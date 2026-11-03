/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use crate::kernel::arch::traits::MemoryArchitecture as _;
use crate::kernel::arch::MemoryArchitecture;
use crate::kernel::memory::physical::PhysicalAllocator;
use crate::kernel::scheduler::sync::IrqSpinLock;

#[derive(Clone, Copy)]
pub(crate) enum FaultAccess {
    Read,
    Write,
    Execute,
}

#[derive(Clone, Copy)]
struct VmArea {
    start_page: usize,
    page_count: usize,
    active: bool,
}

impl VmArea {
    const EMPTY: Self = Self {
        start_page: 0,
        page_count: 0,
        active: false,
    };
}

pub(crate) struct VirtualAreas {
    page_state: [AtomicU8; VirtualAreas::MAX_VM_PAGES],
    mmu_ready: AtomicBool,
    areas: [VmArea; VirtualAreas::MAX_VM_AREAS],
}

impl VirtualAreas {
    pub(crate) const PAGE_SIZE: usize = 4096;

    const VM_ARENA_SIZE: usize =
        <MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::VM_ARENA_SIZE;
    const MAX_VM_PAGES: usize = Self::VM_ARENA_SIZE / Self::PAGE_SIZE;
    const MAX_VM_AREAS: usize = 16384;
    const VM_RESERVED: u8 = 1 << 7;
    const VM_PERMISSION_MASK: u8 = 0x07;
    const VM_BASE: usize =
        <MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::VM_BASE;

    const EMPTY: Self = Self {
        page_state: [const { AtomicU8::new(0) }; Self::MAX_VM_PAGES],
        mmu_ready: AtomicBool::new(false),
        areas: [VmArea::EMPTY; Self::MAX_VM_AREAS],
    };

    fn initialize(&mut self) {
        MemoryArchitecture::initialize_mmu();
        self.mmu_ready.store(true, Ordering::Release);
    }

    fn mmu_ready(&self) -> bool {
        self.mmu_ready.load(Ordering::Acquire)
    }

    pub(crate) const fn allocator_base() -> usize {
        VM_BASE
    }

    fn rounded_length(length: usize) -> Option<usize> {
        length
            .checked_add(Self::PAGE_SIZE - 1)
            .map(|value| value & !(Self::PAGE_SIZE - 1))
    }

    fn align_up(value: usize, alignment: usize) -> Option<usize> {
        value
            .checked_add(alignment - 1)
            .map(|value| value & !(alignment - 1))
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

        let area_slot = self.areas.iter().position(|area| !area.active)?;

        let mut candidate_page = if let Some(address) = hint {
            if address < Self::VM_BASE || address % alignment != 0 {
                return None;
            }
            address.checked_sub(Self::VM_BASE)? / Self::PAGE_SIZE
        } else {
            0
        };
        loop {
            let candidate_address = Self::VM_BASE.checked_add(candidate_page.checked_mul(Self::PAGE_SIZE)?)?;
            let address = if hint.is_some() {
                candidate_address
            } else {
                Self::align_up(candidate_address, alignment)?
            };
            let start_page = address.checked_sub(Self::VM_BASE)? / Self::PAGE_SIZE;
            let end_page = start_page.checked_add(page_count)?;
            if end_page > Self::MAX_VM_PAGES {
                return None;
            }

            let occupied = (start_page..end_page)
                .find(|page| self.page_state[*page].load(Ordering::Acquire) & Self::VM_RESERVED != 0);
            if let Some(occupied_page) = occupied {
                if hint.is_some() {
                    return None;
                }
                candidate_page = occupied_page + 1;
                continue;
            }

            let state_permissions = if permissions == 5 {
                0
            } else {
                permissions as u8
            };
            let state = Self::VM_RESERVED | state_permissions;
            for page in start_page..end_page {
                self.page_state[page].store(state, Ordering::Release);
            }
            self.areas[area_slot] = VmArea {
                start_page,
                page_count,
                active: true,
            };
            return Some(address);
        }
    }

    fn release(&mut self, address: usize, length: usize) -> bool {
        if address < Self::VM_BASE || address % Self::PAGE_SIZE != 0 || length == 0 {
            return false;
        }
        let Some(length) = Self::rounded_length(length) else {
            return false;
        };
        let Some(offset) = address.checked_sub(Self::VM_BASE) else {
            return false;
        };
        let start_page = offset / Self::PAGE_SIZE;
        let page_count = length / Self::PAGE_SIZE;
        let Some(end_page) = start_page.checked_add(page_count) else {
            return false;
        };
        if end_page > Self::MAX_VM_PAGES {
            return false;
        }

        let Some(area_slot) = self.areas.iter().position(|area| {
            area.active && area.start_page == start_page && area.page_count == page_count
        }) else {
            return false;
        };
        if (start_page..end_page)
            .any(|page| self.page_state[page].load(Ordering::Acquire) & Self::VM_RESERVED == 0)
        {
            return false;
        }

        let released = self.release_pages(start_page, end_page);
        self.areas[area_slot] = VmArea::EMPTY;
        released
    }

    fn release_range(&mut self, address: usize, length: usize) -> bool {
        if address < Self::VM_BASE || address % Self::PAGE_SIZE != 0 || length == 0 {
            return false;
        }
        let Some(length) = Self::rounded_length(length) else {
            return false;
        };
        let Some(offset) = address.checked_sub(Self::VM_BASE) else {
            return false;
        };
        let start_page = offset / Self::PAGE_SIZE;
        let page_count = length / Self::PAGE_SIZE;
        let Some(end_page) = start_page.checked_add(page_count) else {
            return false;
        };
        if end_page > Self::MAX_VM_PAGES {
            return false;
        }

        let Some(area_slot) = self.areas.iter().position(|area| {
            area.active
                && start_page >= area.start_page
                && end_page <= area.start_page.saturating_add(area.page_count)
        }) else {
            return false;
        };
        let area = self.areas[area_slot];
        if (start_page..end_page)
            .any(|page| self.page_state[page].load(Ordering::Acquire) & Self::VM_RESERVED == 0)
        {
            return false;
        }

        let area_end = area.start_page + area.page_count;
        let prefix_page_count = start_page - area.start_page;
        let suffix_page_count = area_end - end_page;
        let split_slot = if prefix_page_count != 0 && suffix_page_count != 0 {
            let Some(slot) = self.areas.iter().position(|area| !area.active) else {
                return false;
            };
            Some(slot)
        } else {
            None
        };

        let released = self.release_pages(start_page, end_page);
        match (prefix_page_count, suffix_page_count) {
            (0, 0) => self.areas[area_slot] = VmArea::EMPTY,
            (0, suffix_page_count) => {
                self.areas[area_slot] = VmArea {
                    start_page: end_page,
                    page_count: suffix_page_count,
                    active: true,
                }
            }
            (prefix_page_count, 0) => {
                self.areas[area_slot] = VmArea {
                    start_page: area.start_page,
                    page_count: prefix_page_count,
                    active: true,
                }
            }
            (prefix_page_count, suffix_page_count) => {
                self.areas[area_slot] = VmArea {
                    start_page: area.start_page,
                    page_count: prefix_page_count,
                    active: true,
                };
                self.areas[split_slot.unwrap()] = VmArea {
                    start_page: end_page,
                    page_count: suffix_page_count,
                    active: true,
                };
            }
        }
        released
    }

    fn decommit_debug(code: u8) {
        <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::DebugArchitecture>::
            decommit_debug(code);
    }

    fn decommit(&self, address: usize, length: usize) -> bool {
        if address < Self::VM_BASE || address % Self::PAGE_SIZE != 0 || length == 0 {
            Self::decommit_debug(b'a');
            return false;
        }
        let Some(length) = Self::rounded_length(length) else {
            Self::decommit_debug(b'b');
            return false;
        };
        let Some(offset) = address.checked_sub(VM_BASE) else {
            Self::decommit_debug(b'c');
            return false;
        };
        let start_page = offset / Self::PAGE_SIZE;
        let page_count = length / Self::PAGE_SIZE;
        let Some(end_page) = start_page.checked_add(page_count) else {
            return false;
        };
        if end_page > Self::MAX_VM_PAGES {
            Self::decommit_debug(b'd');
            return false;
        }

        let range_reserved = self.areas.iter().any(|area| {
            area.active
                && start_page >= area.start_page
                && end_page <= area.start_page.saturating_add(area.page_count)
        });
        if !range_reserved {
            Self::decommit_debug(b'e');
            return false;
        }
        if (start_page..end_page)
            .any(|page| self.page_state[page].load(Ordering::Acquire) & Self::VM_RESERVED == 0)
        {
            Self::decommit_debug(b'f');
            return false;
        }

        let mut decommitted = true;
        for page in start_page..end_page {
            let virtual_address = Self::VM_BASE + page * Self::PAGE_SIZE;
            if let Some(physical_address) = MemoryArchitecture::unmap_page(virtual_address) {
                if !PhysicalAllocator::deallocate_contiguous(physical_address, Self::PAGE_SIZE) {
                    decommitted = false;
                    Self::decommit_debug(b'g');
                }
            }
            self.page_state[page].store(Self::VM_RESERVED, Ordering::Release);
        }
        if decommitted {
            Self::decommit_debug(b'.');
        }
        decommitted
    }

    fn release_suffix(&mut self, address: usize, length: usize, new_length: usize) -> bool {
        if address < Self::VM_BASE || address % Self::PAGE_SIZE != 0 || length == 0 || new_length > length {
            return false;
        }
        let Some(length) = Self::rounded_length(length) else {
            return false;
        };
        let Some(new_length) = Self::rounded_length(new_length) else {
            return false;
        };
        if new_length > length {
            return false;
        }
        if new_length == length {
            return true;
        }

        let Some(offset) = address.checked_sub(Self::VM_BASE) else {
            return false;
        };
        let start_page = offset / Self::PAGE_SIZE;
        let old_page_count = length / Self::PAGE_SIZE;
        let new_page_count = new_length / Self::PAGE_SIZE;
        let Some(release_start) = start_page.checked_add(new_page_count) else {
            return false;
        };
        let Some(release_end) = start_page.checked_add(old_page_count) else {
            return false;
        };
        if release_end > Self::MAX_VM_PAGES {
            return false;
        }

        let Some(slot) = self.areas.iter().position(|area| {
            area.active && area.start_page == start_page && area.page_count == old_page_count
        }) else {
            return false;
        };
        if (release_start..release_end)
            .any(|page| self.page_state[page].load(Ordering::Acquire) & Self::VM_RESERVED == 0)
        {
            return false;
        }

        let released = self.release_pages(release_start, release_end);
        if new_page_count == 0 {
            self.areas[slot] = VmArea::EMPTY;
        } else {
            self.areas[slot].page_count = new_page_count;
        }
        released
    }

    fn set_permissions(&self, address: usize, length: usize, permissions: i32) -> bool {
        if address < Self::VM_BASE
            || address % Self::PAGE_SIZE != 0
            || length == 0
            || !(0..=4).contains(&permissions)
        {
            return false;
        }
        let Some(length) = Self::rounded_length(length) else {
            return false;
        };
        let Some(start_page) = address
            .checked_sub(Self::VM_BASE)
            .map(|offset| offset / Self::PAGE_SIZE)
        else {
            return false;
        };
        let page_count = length / Self::PAGE_SIZE;
        let Some(end_page) = start_page.checked_add(page_count) else {
            return false;
        };
        if end_page > Self::MAX_VM_PAGES {
            return false;
        }

        let range_reserved = self.areas.iter().any(|area| {
            area.active
                && start_page >= area.start_page
                && end_page <= area.start_page.saturating_add(area.page_count)
        });
        if !range_reserved {
            return false;
        }

        for page in start_page..end_page {
            let state = self.page_state[page].load(Ordering::Acquire);
            if state & Self::VM_RESERVED == 0 {
                return false;
            }
            let virtual_address = Self::VM_BASE + page * Self::PAGE_SIZE;
            if MemoryArchitecture::page_has_backing(virtual_address)
                && !MemoryArchitecture::set_page_permissions(
                    virtual_address,
                    Self::PAGE_SIZE,
                    permissions,
                )
            {
                return false;
            }
            self.page_state[page].store(Self::VM_RESERVED | permissions as u8, Ordering::Release);
        }
        true
    }

    fn release_pages(&mut self, start_page: usize, end_page: usize) -> bool {
        let mut released = true;
        for page in start_page..end_page {
            let virtual_address = Self::VM_BASE + page * Self::PAGE_SIZE;
            if let Some(physical_address) = MemoryArchitecture::unmap_page(virtual_address) {
                if !PhysicalAllocator::deallocate_contiguous(physical_address, Self::PAGE_SIZE) {
                    released = false;
                }
            }
            self.page_state[page].store(0, Ordering::Release);
        }
        released
    }

    fn resolve_page_fault(&self, address: usize, access: FaultAccess) -> bool {
        let page_address = address & !(Self::PAGE_SIZE - 1);
        let Some(offset) = page_address.checked_sub(Self::VM_BASE) else {
            return false;
        };
        if offset >= Self::VM_ARENA_SIZE {
            return false;
        }
        let page = offset / Self::PAGE_SIZE;

        let state = self.page_state[page].load(Ordering::Acquire);
        if state & Self::VM_RESERVED == 0 {
            return false;
        }
        let permissions = (state & Self::VM_PERMISSION_MASK) as i32;
        let access_allowed = match access {
            FaultAccess::Read => (1..=4).contains(&permissions),
            FaultAccess::Write => matches!(permissions, 2 | 3),
            FaultAccess::Execute => matches!(permissions, 3 | 4),
        };
        if !access_allowed {
            return false;
        }
        if MemoryArchitecture::page_is_mapped(page_address) {
            return true;
        }

        let Some(frame) = PhysicalAllocator::allocate() else {
            return false;
        };
        let physical_address = frame.address();
        if !MemoryArchitecture::set_page_permissions(physical_address, Self::PAGE_SIZE, 2) {
            let _released = PhysicalAllocator::deallocate(frame);
            return false;
        }
        unsafe { core::ptr::write_bytes(physical_address as *mut u8, 0, Self::PAGE_SIZE) };
        if MemoryArchitecture::map_page(page_address, physical_address, permissions) {
            return true;
        }
        let _released = PhysicalAllocator::deallocate(frame);
        false
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

    pub(crate) fn decommit(address: usize, length: usize) -> bool {
        Self::get_instance().lock().decommit(address, length)
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

    pub(crate) fn is_identity_mapped(address: usize) -> bool {
        MemoryArchitecture::is_identity_mapped(address)
    }

    pub(crate) fn translate(address: usize) -> Option<usize> {
        Self::is_identity_mapped(address).then_some(address)
    }
}

pub(crate) const PAGE_SIZE: usize = VirtualAreas::PAGE_SIZE;

pub(super) fn initialize() {
    VirtualAreasLock::initialize();
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

pub(crate) fn decommit(address: usize, length: usize) -> bool {
    VirtualAreasLock::decommit(address, length)
}

pub(crate) fn release_suffix(address: usize, length: usize, new_length: usize) -> bool {
    VirtualAreasLock::release_suffix(address, length, new_length)
}

pub(crate) fn set_permissions(address: usize, length: usize, permissions: i32) -> bool {
    VirtualAreasLock::set_permissions(address, length, permissions)
}

pub(crate) fn resolve_page_fault(address: usize, access: FaultAccess) -> bool {
    VirtualAreasLock::resolve_page_fault(address, access)
}

pub(crate) fn is_identity_mapped(address: usize) -> bool {
    VirtualAreasLock::is_identity_mapped(address)
}

pub(crate) fn translate(address: usize) -> Option<usize> {
    VirtualAreasLock::translate(address)
}