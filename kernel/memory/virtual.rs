/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ops::Range;
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};

use crate::kernel::arch::traits::{DebugArchitecture as _, MemoryArchitecture as _};
use crate::kernel::arch::MemoryArchitecture;
use crate::kernel::lib::interval::IntervalTree;
use crate::kernel::memory::physical::PhysicalAllocator;
use crate::kernel::scheduler::sync::IrqSpinLock;

#[derive(Clone, Copy)]
pub(crate) enum FaultAccess {
    Read,
    Write,
    Execute,
}

#[derive(Clone, Copy)]
struct PageRange {
    start_page: usize,
    end_page: usize,
}

#[derive(Clone, Copy)]
enum PageRangeError {
    InvalidRange,
    LengthOverflow,
    OffsetUnderflow,
    RangeOverflow,
    OutOfBounds,
}

pub(crate) struct VirtualAreas {
    page_state: [AtomicU16; VirtualAreas::MAX_VM_PAGES],
    mmu_ready: AtomicBool,
    area_index: IntervalTree<(), { VirtualAreas::MAX_VM_AREAS }>,
}

impl VirtualAreas {
    pub(crate) const PAGE_SIZE: usize = 4096;

    const VM_ARENA_SIZE: usize =
        <MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::VM_ARENA_SIZE;
    const MAX_VM_PAGES: usize = Self::VM_ARENA_SIZE / Self::PAGE_SIZE;
    const MAX_VM_AREAS: usize = 16384;
    const VM_RESERVED: u16 = 1 << 7;
    const VM_OWNED: u16 = 1 << 8;
    const VM_BORROWED: u16 = 1 << 9;
    const VM_PERMISSION_MASK: u16 = 0x07;
    const VM_BASE: usize =
        <MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::VM_BASE;

    const EMPTY: Self = Self {
        page_state: [const { AtomicU16::new(0) }; Self::MAX_VM_PAGES],
        mmu_ready: AtomicBool::new(false),
        area_index: IntervalTree::new(),
    };

    fn initialize(&mut self) {
        MemoryArchitecture::initialize_mmu();
        self.mmu_ready.store(true, Ordering::Release);
    }

    fn mmu_ready(&self) -> bool {
        self.mmu_ready.load(Ordering::Acquire)
    }

    pub(crate) const fn allocator_base() -> usize {
        Self::VM_BASE
    }

    fn rounded_length(length: usize) -> Option<usize> {
        length
            .checked_add(Self::PAGE_SIZE - 1)
            .map(|value| value & !(Self::PAGE_SIZE - 1))
    }

    fn checked_page_range(
        address: usize,
        length: usize,
    ) -> Result<PageRange, PageRangeError> {
        if length == 0 {
            return Err(PageRangeError::InvalidRange);
        }
        let page_count = Self::rounded_page_count(length).ok_or(PageRangeError::LengthOverflow)?;
        let start_page = Self::page_start(address)?;
        Self::checked_page_range_from_start(start_page, page_count)
    }

    fn rounded_page_count(length: usize) -> Option<usize> {
        Self::rounded_length(length).map(|length| length / Self::PAGE_SIZE)
    }

    fn page_start(address: usize) -> Result<usize, PageRangeError> {
        if address < Self::VM_BASE || address % Self::PAGE_SIZE != 0 {
            return Err(PageRangeError::InvalidRange);
        }
        let offset = address
            .checked_sub(Self::VM_BASE)
            .ok_or(PageRangeError::OffsetUnderflow)?;
        Ok(offset / Self::PAGE_SIZE)
    }

    fn checked_page_range_from_start(
        start_page: usize,
        page_count: usize,
    ) -> Result<PageRange, PageRangeError> {
        let end_page = start_page
            .checked_add(page_count)
            .ok_or(PageRangeError::RangeOverflow)?;
        if end_page > Self::MAX_VM_PAGES {
            return Err(PageRangeError::OutOfBounds);
        }
        Ok(PageRange {
            start_page,
            end_page,
        })
    }

    fn address_range(range: PageRange) -> Option<Range<usize>> {
        let start_offset = range.start_page.checked_mul(Self::PAGE_SIZE)?;
        let end_offset = range.end_page.checked_mul(Self::PAGE_SIZE)?;
        Some(
            Self::VM_BASE.checked_add(start_offset)?
                ..Self::VM_BASE.checked_add(end_offset)?,
        )
    }

    fn exact_area(&self, range: PageRange) -> bool {
        let Some(address_range) = Self::address_range(range) else {
            return false;
        };
        self.area_index.get(&address_range).is_some()
    }

    fn containing_area(&self, range: PageRange) -> Option<PageRange> {
        let address_range = Self::address_range(range)?;
        let (area_range, _) = self.area_index.find_point(address_range.start)?;
        if address_range.end > area_range.end {
            return None;
        }
        Some(PageRange {
            start_page: Self::page_start(area_range.start).ok()?,
            end_page: Self::page_start(area_range.end).ok()?,
        })
    }

    fn reserve(&mut self, length: usize, alignment: usize, permissions: i32) -> Option<usize> {
        self.reserve_with_hint(None, length, alignment, permissions)
    }

    fn reserve_with_hint(
        &mut self,
        hint: Option<usize>,
        length: usize,
        alignment: usize,
        permissions: i32,
    ) -> Option<usize> {
        if length == 0
            || alignment == 0
            || !alignment.is_power_of_two()
            || !(0..=5).contains(&permissions)
        {
            return None;
        }
        let length = Self::rounded_length(length)?;
        let page_count = length / Self::PAGE_SIZE;
        if page_count > Self::MAX_VM_PAGES {
            return None;
        }
        let alignment = alignment.max(Self::PAGE_SIZE);

        if self.area_index.is_full() {
            return None;
        }
        let address_range = if let Some(address) = hint {
            if address < Self::VM_BASE || address % alignment != 0 {
                return None;
            }
            let range = Self::checked_page_range(address, length).ok()?;
            let address_range = Self::address_range(range)?;
            if self.area_index.find_overlap(address_range.clone()).is_some() {
                return None;
            }
            address_range
        } else {
            let arena_end = Self::VM_BASE.checked_add(Self::VM_ARENA_SIZE)?;
            self.area_index
                .first_fit(Self::VM_BASE..arena_end, length, alignment)?
        };

        let range = Self::checked_page_range(address_range.start, length).ok()?;
        let start_page = range.start_page;
        let end_page = range.end_page;
        let address = address_range.start;
        if self.area_index.insert(address_range, ()).is_err() {
            return None;
        }

        let state_permissions = if permissions == 5 {
            0
        } else {
            permissions as u16
        };
        let state = Self::VM_RESERVED | state_permissions;
        for page in start_page..end_page {
            self.page_state[page].store(state, Ordering::Release);
        }
        Some(address)
    }

    fn release(&mut self, address: usize, length: usize) -> bool {
        let Ok(range) = Self::checked_page_range(address, length) else {
            return false;
        };
        let Some(address_range) = Self::address_range(range) else {
            return false;
        };
        if !self.exact_area(range) {
            return false;
        }
        if (range.start_page..range.end_page)
            .any(|page| self.page_state[page].load(Ordering::Acquire) & Self::VM_RESERVED == 0)
        {
            return false;
        }

        if self.area_index.remove(&address_range).is_none() {
            return false;
        }
        let released = self.release_pages(range.start_page, range.end_page);
        released
    }

    fn release_range(&mut self, address: usize, length: usize) -> bool {
        self.release_range_inner(address, length)
            .is_some_and(|(_, _, released)| released)
    }

    fn release_range_inner(
        &mut self,
        address: usize,
        length: usize,
    ) -> Option<(u64, u64, bool)> {
        let Ok(range) = Self::checked_page_range(address, length) else {
            return None;
        };
        let Some(area) = self.containing_area(range) else {
            return None;
        };
        if (range.start_page..range.end_page)
            .any(|page| self.page_state[page].load(Ordering::Acquire) & Self::VM_RESERVED == 0)
        {
            return None;
        }

        let area_address_range = Self::address_range(area).expect("valid VM area range");
        let prefix_page_count = range.start_page - area.start_page;
        let area_end = area.end_page;
        let suffix_page_count = area_end - range.end_page;
        if prefix_page_count != 0 && suffix_page_count != 0 && self.area_index.is_full() {
            return None;
        }

        if self.area_index.remove(&area_address_range).is_none() {
            return None;
        }
        match (prefix_page_count, suffix_page_count) {
            (0, 0) => {}
            (0, _) => {
                let suffix = PageRange {
                    start_page: range.end_page,
                    end_page: area_end,
                };
                let address_range = Self::address_range(suffix).expect("valid VM suffix range");
                let inserted = self.area_index.insert(address_range, ());
                debug_assert!(matches!(inserted, Ok(None)));
            }
            (_, 0) => {
                let prefix = PageRange {
                    start_page: area.start_page,
                    end_page: range.start_page,
                };
                let address_range = Self::address_range(prefix).expect("valid VM prefix range");
                let inserted = self.area_index.insert(address_range, ());
                debug_assert!(matches!(inserted, Ok(None)));
            }
            (_, _) => {
                let prefix = PageRange {
                    start_page: area.start_page,
                    end_page: range.start_page,
                };
                let prefix_address_range =
                    Self::address_range(prefix).expect("valid VM prefix range");
                let prefix_inserted = self.area_index.insert(prefix_address_range, ());
                debug_assert!(matches!(prefix_inserted, Ok(None)));

                let suffix = PageRange {
                    start_page: range.end_page,
                    end_page: area.end_page,
                };
                let suffix_address_range =
                    Self::address_range(suffix).expect("valid VM suffix range");
                let suffix_inserted = self.area_index.insert(suffix_address_range, ());
                debug_assert!(matches!(suffix_inserted, Ok(None)));
            }
        }
        let resident_pages = (range.start_page..range.end_page)
            .filter(|page| {
                self.page_state[*page].load(Ordering::Acquire) & Self::VM_OWNED != 0
            })
            .count();
        let released = self.release_pages(range.start_page, range.end_page);
        Some((
            (range.end_page - range.start_page) as u64 * Self::PAGE_SIZE as u64,
            resident_pages as u64 * Self::PAGE_SIZE as u64,
            released,
        ))
    }

    fn decommit_debug(code: u8) {
        crate::kernel::arch::DebugArchitecture::decommit_debug(code);
    }

    fn decommit(&self, address: usize, length: usize) -> bool {
        self.decommit_accounted(address, length)
            .is_some_and(|(_, decommitted)| decommitted)
    }

    fn decommit_accounted(&self, address: usize, length: usize) -> Option<(u64, bool)> {
        let range = match Self::checked_page_range(address, length) {
            Ok(range) => range,
            Err(PageRangeError::InvalidRange) => {
                Self::decommit_debug(b'a');
                return None;
            }
            Err(PageRangeError::LengthOverflow) => {
                Self::decommit_debug(b'b');
                return None;
            }
            Err(PageRangeError::OffsetUnderflow) => {
                Self::decommit_debug(b'c');
                return None;
            }
            Err(PageRangeError::RangeOverflow) => return None,
            Err(PageRangeError::OutOfBounds) => {
                Self::decommit_debug(b'd');
                return None;
            }
        };

        if self.containing_area(range).is_none() {
            Self::decommit_debug(b'e');
            return None;
        }
        if (range.start_page..range.end_page)
            .any(|page| self.page_state[page].load(Ordering::Acquire) & Self::VM_RESERVED == 0)
        {
            Self::decommit_debug(b'f');
            return None;
        }

        let resident_pages = (range.start_page..range.end_page)
            .filter(|page| {
                self.page_state[*page].load(Ordering::Acquire) & Self::VM_OWNED != 0
            })
            .count();
        let mut decommitted = true;
        for page in range.start_page..range.end_page {
            let virtual_address = Self::VM_BASE + page * Self::PAGE_SIZE;
            let state = self.page_state[page].load(Ordering::Acquire);
            if let Some(physical_address) = MemoryArchitecture::unmap_page(virtual_address) {
                if state & Self::VM_BORROWED == 0
                    && !PhysicalAllocator::deallocate_contiguous(physical_address, Self::PAGE_SIZE)
                {
                    decommitted = false;
                    Self::decommit_debug(b'g');
                }
            }
            self.page_state[page].store(Self::VM_RESERVED, Ordering::Release);
        }
        if decommitted {
            Self::decommit_debug(b'.');
        }
        Some((resident_pages as u64 * Self::PAGE_SIZE as u64, decommitted))
    }

    fn release_suffix(&mut self, address: usize, length: usize, new_length: usize) -> bool {
        if length == 0 || new_length > length {
            return false;
        }
        let Some(old_page_count) = Self::rounded_page_count(length) else {
            return false;
        };
        let Some(new_page_count) = Self::rounded_page_count(new_length) else {
            return false;
        };
        if new_page_count > old_page_count {
            return false;
        }
        let Ok(start_page) = Self::page_start(address) else {
            return false;
        };
        if new_page_count == old_page_count {
            return true;
        }

        let Ok(range) = Self::checked_page_range_from_start(start_page, old_page_count) else {
            return false;
        };
        let Some(release_start) = start_page.checked_add(new_page_count) else {
            return false;
        };
        let release_end = range.end_page;
        if !self.exact_area(range) {
            return false;
        }
        if (release_start..release_end)
            .any(|page| self.page_state[page].load(Ordering::Acquire) & Self::VM_RESERVED == 0)
        {
            return false;
        }

        let Some(address_range) = Self::address_range(range) else {
            return false;
        };
        if self.area_index.remove(&address_range).is_none() {
            return false;
        }
        if new_page_count != 0 {
            let shortened = PageRange {
                start_page,
                end_page: release_start,
            };
            let shortened_range = Self::address_range(shortened).expect("valid shortened range");
            let inserted = self.area_index.insert(shortened_range, ());
            debug_assert!(matches!(inserted, Ok(None)));
        }

        let released = self.release_pages(release_start, release_end);
        released
    }

    fn set_permissions(&self, address: usize, length: usize, permissions: i32) -> bool {
        if !(0..=4).contains(&permissions) {
            return false;
        }
        let Ok(range) = Self::checked_page_range(address, length) else {
            return false;
        };
        if self.containing_area(range).is_none() {
            return false;
        }

        for page in range.start_page..range.end_page {
            let state = self.page_state[page].load(Ordering::Acquire);
            if state & Self::VM_RESERVED == 0 {
                return false;
            }
            let virtual_address = Self::VM_BASE + page * Self::PAGE_SIZE;
            if MemoryArchitecture::page_has_backing(virtual_address) {
                if !MemoryArchitecture::set_page_permissions(
                    virtual_address,
                    Self::PAGE_SIZE,
                    permissions,
                ) {
                    return false;
                }
            }
            self.page_state[page].store(
                Self::VM_RESERVED
                    | (state & (Self::VM_OWNED | Self::VM_BORROWED))
                    | permissions as u16,
                Ordering::Release,
            );
        }
        true
    }

    fn release_pages(&mut self, start_page: usize, end_page: usize) -> bool {
        let mut released = true;
        for page in start_page..end_page {
            let virtual_address = Self::VM_BASE + page * Self::PAGE_SIZE;
            let state = self.page_state[page].load(Ordering::Acquire);
            if let Some(physical_address) = MemoryArchitecture::unmap_page(virtual_address) {
                if state & Self::VM_BORROWED == 0
                    && !PhysicalAllocator::deallocate_contiguous(physical_address, Self::PAGE_SIZE)
                {
                    released = false;
                }
            }
            self.page_state[page].store(0, Ordering::Release);
        }
        released
    }

    fn resolve_page_fault(&self, address: usize, access: FaultAccess) -> bool {
        self.resolve_page_fault_accounted(address, access).0
    }

    fn resolve_page_fault_accounted(
        &self,
        address: usize,
        access: FaultAccess,
    ) -> (bool, bool) {
        let page_address = address & !(Self::PAGE_SIZE - 1);
        let Some(offset) = page_address.checked_sub(Self::VM_BASE) else {
            return (false, false);
        };
        if offset >= Self::VM_ARENA_SIZE {
            return (false, false);
        }
        let page = offset / Self::PAGE_SIZE;

        let state = self.page_state[page].load(Ordering::Acquire);
        if state & Self::VM_RESERVED == 0 {
            return (false, false);
        }
        let permissions = (state & Self::VM_PERMISSION_MASK) as i32;
        let access_allowed = match access {
            FaultAccess::Read => (1..=4).contains(&permissions),
            FaultAccess::Write => matches!(permissions, 2 | 3),
            FaultAccess::Execute => matches!(permissions, 3 | 4),
        };
        if !access_allowed {
            return (false, false);
        }
        if MemoryArchitecture::page_is_mapped(page_address) {
            return (true, false);
        }

        let Some(frame) = PhysicalAllocator::allocate() else {
            return (false, false);
        };
        let physical_address = frame.address();
        if !MemoryArchitecture::map_page(page_address, physical_address, 2) {
            let _released = PhysicalAllocator::deallocate(frame);
            return (false, false);
        }
        unsafe { core::ptr::write_bytes(page_address as *mut u8, 0, Self::PAGE_SIZE) };
        if permissions == 2
            || MemoryArchitecture::set_page_permissions(page_address, Self::PAGE_SIZE, permissions)
        {
            self.page_state[page].fetch_or(Self::VM_OWNED, Ordering::Release);
            return (true, true);
        }
        let _unmapped = MemoryArchitecture::unmap_page(page_address);
        let _released = PhysicalAllocator::deallocate(frame);
        (false, false)
    }
}

struct VirtualAreasLock;

impl VirtualAreasLock {
    pub(crate) fn get_instance() -> &'static IrqSpinLock<VirtualAreas> {
        static VM_AREAS: IrqSpinLock<VirtualAreas> = IrqSpinLock::new(VirtualAreas::EMPTY);
        &VM_AREAS
    }

    fn initialize() {
        Self::get_instance().lock().initialize();
    }

    fn mmu_ready() -> bool {
        Self::get_instance().lock().mmu_ready()
    }

    pub(crate) fn reserve(length: usize, alignment: usize, permissions: i32) -> Option<usize> {
        Self::get_instance().lock().reserve(length, alignment, permissions)
    }

    pub(crate) fn reserve_with_hint(
        hint: Option<usize>,
        length: usize,
        alignment: usize,
        permissions: i32,
    ) -> Option<usize> {
        Self::get_instance()
            .lock()
            .reserve_with_hint(hint, length, alignment, permissions)
    }

    pub(crate) fn release(address: usize, length: usize) -> bool {
        Self::get_instance().lock().release(address, length)
    }

    pub(crate) fn release_range(address: usize, length: usize) -> bool {
        Self::get_instance().lock().release_range(address, length)
    }

    pub(crate) fn release_range_accounted(
        address: usize,
        length: usize,
    ) -> Option<(u64, u64)> {
        Self::get_instance()
            .lock()
            .release_range_inner(address, length)
            .map(|(address_space_bytes, resident_bytes, _)| {
                (address_space_bytes, resident_bytes)
            })
    }

    pub(crate) fn decommit(address: usize, length: usize) -> bool {
        Self::get_instance().lock().decommit(address, length)
    }

    pub(crate) fn decommit_accounted(address: usize, length: usize) -> Option<(u64, bool)> {
        Self::get_instance()
            .lock()
            .decommit_accounted(address, length)
    }

    pub(crate) fn release_suffix(address: usize, length: usize, new_length: usize) -> bool {
        Self::get_instance().lock().release_suffix(address, length, new_length)
    }

    pub(crate) fn set_permissions(address: usize, length: usize, permissions: i32) -> bool {
        Self::get_instance()
            .lock()
            .set_permissions(address, length, permissions)
    }

    pub(crate) fn resolve_page_fault(address: usize, access: FaultAccess) -> bool {
        Self::get_instance().lock().resolve_page_fault(address, access)
    }

    pub(crate) fn resolve_page_fault_accounted(
        address: usize,
        access: FaultAccess,
    ) -> (bool, bool) {
        Self::get_instance()
            .lock()
            .resolve_page_fault_accounted(address, access)
    }

    #[allow(dead_code)]
    pub(crate) fn is_identity_mapped(address: usize) -> bool {
        MemoryArchitecture::is_identity_mapped(address)
    }

    #[allow(dead_code)]
    pub(crate) fn translate(address: usize) -> Option<usize> {
        Self::is_identity_mapped(address).then_some(address)
    }
}

pub(crate) const PAGE_SIZE: usize = VirtualAreas::PAGE_SIZE;

pub(crate) fn initialize() {
    VirtualAreasLock::initialize();
}

pub(crate) fn refresh_memory_map() {
    MemoryArchitecture::initialize_mmu();
}

pub(crate) fn mmu_ready() -> bool {
    VirtualAreasLock::mmu_ready()
}

pub(crate) const fn allocator_base() -> usize {
    VirtualAreas::allocator_base()
}

pub(crate) fn reserve(length: usize, alignment: usize, permissions: i32) -> Option<usize> {
    VirtualAreasLock::reserve(length, alignment, permissions)
}

pub(crate) fn reserve_with_hint(
    hint: Option<usize>,
    length: usize,
    alignment: usize,
    permissions: i32,
) -> Option<usize> {
    VirtualAreasLock::reserve_with_hint(hint, length, alignment, permissions)
}

pub(crate) fn release(address: usize, length: usize) -> bool {
    VirtualAreasLock::release(address, length)
}

pub(crate) fn release_range(address: usize, length: usize) -> bool {
    VirtualAreasLock::release_range(address, length)
}

pub(crate) fn release_range_accounted(address: usize, length: usize) -> Option<(u64, u64)> {
    VirtualAreasLock::release_range_accounted(address, length)
}

pub(crate) fn decommit(address: usize, length: usize) -> bool {
    VirtualAreasLock::decommit(address, length)
}

pub(crate) fn decommit_accounted(address: usize, length: usize) -> Option<(u64, bool)> {
    VirtualAreasLock::decommit_accounted(address, length)
}

pub(crate) fn release_suffix(address: usize, length: usize, new_length: usize) -> bool {
    VirtualAreasLock::release_suffix(address, length, new_length)
}

pub(crate) fn set_permissions(address: usize, length: usize, permissions: i32) -> bool {
    VirtualAreasLock::set_permissions(address, length, permissions)
}

pub(crate) fn resolve_page_fault(address: usize, access: FaultAccess) -> bool {
    let process = crate::kernel::scheduler::thread::ThreadManager::current_process().ok();
    let (resolved, newly_resident) = match address.checked_sub(VirtualAreas::allocator_base()) {
        Some(offset) if offset < VirtualAreas::VM_ARENA_SIZE => {
            VirtualAreasLock::resolve_page_fault_accounted(address, access)
        }
        _ => process.as_ref().map_or((false, false), |process| {
            process
                .address_space()
                .resolve_page_fault_accounted(address, access)
        }),
    };
    if resolved && newly_resident {
        if let Some(process) = process {
            process.record_page_fault(false);
        }
    }
    resolved
}

#[allow(dead_code)]
pub(crate) fn is_identity_mapped(address: usize) -> bool {
    VirtualAreasLock::is_identity_mapped(address)
}

#[allow(dead_code)]
pub(crate) fn translate(address: usize) -> Option<usize> {
    VirtualAreasLock::translate(address)
}