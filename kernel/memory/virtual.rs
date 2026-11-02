/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use crate::sync::IrqSpinLock;

pub(crate) const PAGE_SIZE: usize = 4096;

const VM_ARENA_SIZE: usize = crate::arch::VM_ARENA_SIZE;
const MAX_VM_PAGES: usize = VM_ARENA_SIZE / PAGE_SIZE;
const MAX_VM_AREAS: usize = 16384;
const VM_RESERVED: u8 = 1 << 7;
const VM_PERMISSION_MASK: u8 = 0x07;

const VM_BASE: usize = crate::arch::VM_BASE;

static PAGE_STATE: [AtomicU8; MAX_VM_PAGES] = [const { AtomicU8::new(0) }; MAX_VM_PAGES];
static MMU_READY: AtomicBool = AtomicBool::new(false);

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

static VM_AREAS: IrqSpinLock<[VmArea; MAX_VM_AREAS]> =
    IrqSpinLock::new([VmArea::EMPTY; MAX_VM_AREAS]);

#[derive(Clone, Copy)]
pub(crate) enum FaultAccess {
    Read,
    Write,
    Execute,
}

pub(super) fn initialize() {
    crate::arch::initialize_mmu();
    MMU_READY.store(true, Ordering::Release);
}

pub(crate) fn mmu_ready() -> bool {
    MMU_READY.load(Ordering::Acquire)
}

pub(crate) const fn allocator_base() -> usize {
    VM_BASE
}

fn rounded_length(length: usize) -> Option<usize> {
    length
        .checked_add(PAGE_SIZE - 1)
        .map(|value| value & !(PAGE_SIZE - 1))
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
}

pub(crate) fn reserve(length: usize, alignment: usize, permissions: i32) -> Option<usize> {
    reserve_with_hint(None, length, alignment, permissions)
}

pub(crate) fn reserve_with_hint(
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
    let length = rounded_length(length)?;
    let page_count = length / PAGE_SIZE;
    if page_count > MAX_VM_PAGES {
        return None;
    }
    let alignment = alignment.max(PAGE_SIZE);

    let mut areas = VM_AREAS.lock();
    let area_slot = areas.iter().position(|area| !area.active)?;

    let mut candidate_page = if let Some(address) = hint {
        if address < VM_BASE || address % alignment != 0 {
            return None;
        }
        address.checked_sub(VM_BASE)? / PAGE_SIZE
    } else {
        0
    };
    loop {
        let candidate_address = VM_BASE.checked_add(candidate_page.checked_mul(PAGE_SIZE)?)?;
        let address = if hint.is_some() {
            candidate_address
        } else {
            align_up(candidate_address, alignment)?
        };
        let start_page = address.checked_sub(VM_BASE)? / PAGE_SIZE;
        let end_page = start_page.checked_add(page_count)?;
        if end_page > MAX_VM_PAGES {
            return None;
        }

        let occupied = (start_page..end_page)
            .find(|page| PAGE_STATE[*page].load(Ordering::Acquire) & VM_RESERVED != 0);
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
        let state = VM_RESERVED | state_permissions;
        for page in start_page..end_page {
            PAGE_STATE[page].store(state, Ordering::Release);
        }
        areas[area_slot] = VmArea {
            start_page,
            page_count,
            active: true,
        };
        return Some(address);
    }
}

pub(crate) fn release(address: usize, length: usize) -> bool {
    if address < VM_BASE || address % PAGE_SIZE != 0 || length == 0 {
        return false;
    }
    let Some(length) = rounded_length(length) else {
        return false;
    };
    let Some(offset) = address.checked_sub(VM_BASE) else {
        return false;
    };
    let start_page = offset / PAGE_SIZE;
    let page_count = length / PAGE_SIZE;
    let Some(end_page) = start_page.checked_add(page_count) else {
        return false;
    };
    if end_page > MAX_VM_PAGES {
        return false;
    }

    let mut areas = VM_AREAS.lock();
    let Some(area_slot) = areas.iter().position(|area| {
        area.active && area.start_page == start_page && area.page_count == page_count
    }) else {
        return false;
    };
    if (start_page..end_page)
        .any(|page| PAGE_STATE[page].load(Ordering::Acquire) & VM_RESERVED == 0)
    {
        return false;
    }

    let released = release_pages(start_page, end_page);
    areas[area_slot] = VmArea::EMPTY;
    released
}

pub(crate) fn release_range(address: usize, length: usize) -> bool {
    if address < VM_BASE || address % PAGE_SIZE != 0 || length == 0 {
        return false;
    }
    let Some(length) = rounded_length(length) else {
        return false;
    };
    let Some(offset) = address.checked_sub(VM_BASE) else {
        return false;
    };
    let start_page = offset / PAGE_SIZE;
    let page_count = length / PAGE_SIZE;
    let Some(end_page) = start_page.checked_add(page_count) else {
        return false;
    };
    if end_page > MAX_VM_PAGES {
        return false;
    }

    let mut areas = VM_AREAS.lock();
    let Some(area_slot) = areas.iter().position(|area| {
        area.active
            && start_page >= area.start_page
            && end_page <= area.start_page.saturating_add(area.page_count)
    }) else {
        return false;
    };
    let area = areas[area_slot];
    if (start_page..end_page)
        .any(|page| PAGE_STATE[page].load(Ordering::Acquire) & VM_RESERVED == 0)
    {
        return false;
    }

    let area_end = area.start_page + area.page_count;
    let prefix_page_count = start_page - area.start_page;
    let suffix_page_count = area_end - end_page;
    let split_slot = if prefix_page_count != 0 && suffix_page_count != 0 {
        let Some(slot) = areas.iter().position(|area| !area.active) else {
            return false;
        };
        Some(slot)
    } else {
        None
    };

    let released = release_pages(start_page, end_page);
    match (prefix_page_count, suffix_page_count) {
        (0, 0) => areas[area_slot] = VmArea::EMPTY,
        (0, suffix_page_count) => {
            areas[area_slot] = VmArea {
                start_page: end_page,
                page_count: suffix_page_count,
                active: true,
            }
        }
        (prefix_page_count, 0) => {
            areas[area_slot] = VmArea {
                start_page: area.start_page,
                page_count: prefix_page_count,
                active: true,
            }
        }
        (prefix_page_count, suffix_page_count) => {
            areas[area_slot] = VmArea {
                start_page: area.start_page,
                page_count: prefix_page_count,
                active: true,
            };
            areas[split_slot.unwrap()] = VmArea {
                start_page: end_page,
                page_count: suffix_page_count,
                active: true,
            };
        }
    }
    released
}

fn decommit_debug(code: u8) {
    crate::arch::decommit_debug(code);
}

pub(crate) fn decommit(address: usize, length: usize) -> bool {
    if address < VM_BASE || address % PAGE_SIZE != 0 || length == 0 {
        decommit_debug(b'a');
        return false;
    }
    let Some(length) = rounded_length(length) else {
        decommit_debug(b'b');
        return false;
    };
    let Some(offset) = address.checked_sub(VM_BASE) else {
        decommit_debug(b'c');
        return false;
    };
    let start_page = offset / PAGE_SIZE;
    let page_count = length / PAGE_SIZE;
    let Some(end_page) = start_page.checked_add(page_count) else {
        return false;
    };
    if end_page > MAX_VM_PAGES {
        decommit_debug(b'd');
        return false;
    }

    let _areas = VM_AREAS.lock();
    let range_reserved = _areas.iter().any(|area| {
        area.active
            && start_page >= area.start_page
            && end_page <= area.start_page.saturating_add(area.page_count)
    });
    if !range_reserved {
        decommit_debug(b'e');
        return false;
    }
    if (start_page..end_page)
        .any(|page| PAGE_STATE[page].load(Ordering::Acquire) & VM_RESERVED == 0)
    {
        decommit_debug(b'f');
        return false;
    }

    let mut decommitted = true;
    for page in start_page..end_page {
        let virtual_address = VM_BASE + page * PAGE_SIZE;
        if let Some(physical_address) = crate::arch::unmap_page(virtual_address) {
            if !crate::physical::deallocate_contiguous(physical_address, PAGE_SIZE) {
                decommitted = false;
                decommit_debug(b'g');
            }
        }
        PAGE_STATE[page].store(VM_RESERVED, Ordering::Release);
    }
    if decommitted {
        decommit_debug(b'.');
    }
    decommitted
}

pub(crate) fn release_suffix(address: usize, length: usize, new_length: usize) -> bool {
    if address < VM_BASE || address % PAGE_SIZE != 0 || length == 0 || new_length > length {
        return false;
    }
    let Some(length) = rounded_length(length) else {
        return false;
    };
    let Some(new_length) = rounded_length(new_length) else {
        return false;
    };
    if new_length > length {
        return false;
    }
    if new_length == length {
        return true;
    }

    let Some(offset) = address.checked_sub(VM_BASE) else {
        return false;
    };
    let start_page = offset / PAGE_SIZE;
    let old_page_count = length / PAGE_SIZE;
    let new_page_count = new_length / PAGE_SIZE;
    let Some(release_start) = start_page.checked_add(new_page_count) else {
        return false;
    };
    let Some(release_end) = start_page.checked_add(old_page_count) else {
        return false;
    };
    if release_end > MAX_VM_PAGES {
        return false;
    }

    let mut areas = VM_AREAS.lock();
    let Some(slot) = areas.iter().position(|area| {
        area.active && area.start_page == start_page && area.page_count == old_page_count
    }) else {
        return false;
    };
    if (release_start..release_end)
        .any(|page| PAGE_STATE[page].load(Ordering::Acquire) & VM_RESERVED == 0)
    {
        return false;
    }

    let released = release_pages(release_start, release_end);
    if new_page_count == 0 {
        areas[slot] = VmArea::EMPTY;
    } else {
        areas[slot].page_count = new_page_count;
    }
    released
}

pub(crate) fn set_permissions(address: usize, length: usize, permissions: i32) -> bool {
    if address < VM_BASE
        || address % PAGE_SIZE != 0
        || length == 0
        || !(0..=4).contains(&permissions)
    {
        return false;
    }
    let Some(length) = rounded_length(length) else {
        return false;
    };
    let Some(start_page) = address
        .checked_sub(VM_BASE)
        .map(|offset| offset / PAGE_SIZE)
    else {
        return false;
    };
    let page_count = length / PAGE_SIZE;
    let Some(end_page) = start_page.checked_add(page_count) else {
        return false;
    };
    if end_page > MAX_VM_PAGES {
        return false;
    }

    let areas = VM_AREAS.lock();
    let range_reserved = areas.iter().any(|area| {
        area.active
            && start_page >= area.start_page
            && end_page <= area.start_page.saturating_add(area.page_count)
    });
    if !range_reserved {
        return false;
    }

    for page in start_page..end_page {
        let state = PAGE_STATE[page].load(Ordering::Acquire);
        if state & VM_RESERVED == 0 {
            return false;
        }
        let virtual_address = VM_BASE + page * PAGE_SIZE;
        if crate::arch::page_has_backing(virtual_address)
            && !crate::arch::set_page_permissions(virtual_address, PAGE_SIZE, permissions)
        {
            return false;
        }
        PAGE_STATE[page].store(VM_RESERVED | permissions as u8, Ordering::Release);
    }
    true
}

fn release_pages(start_page: usize, end_page: usize) -> bool {
    let mut released = true;
    for page in start_page..end_page {
        let virtual_address = VM_BASE + page * PAGE_SIZE;
        if let Some(physical_address) = crate::arch::unmap_page(virtual_address) {
            if !crate::physical::deallocate_contiguous(physical_address, PAGE_SIZE) {
                released = false;
            }
        }
        PAGE_STATE[page].store(0, Ordering::Release);
    }
    released
}

pub(crate) fn resolve_page_fault(address: usize, access: FaultAccess) -> bool {
    let page_address = address & !(PAGE_SIZE - 1);
    let Some(offset) = page_address.checked_sub(VM_BASE) else {
        return false;
    };
    if offset >= VM_ARENA_SIZE {
        return false;
    }
    let page = offset / PAGE_SIZE;

    let _areas = VM_AREAS.lock();
    let state = PAGE_STATE[page].load(Ordering::Acquire);
    if state & VM_RESERVED == 0 {
        return false;
    }
    let permissions = (state & VM_PERMISSION_MASK) as i32;
    let access_allowed = match access {
        FaultAccess::Read => (1..=4).contains(&permissions),
        FaultAccess::Write => matches!(permissions, 2 | 3),
        FaultAccess::Execute => matches!(permissions, 3 | 4),
    };
    if !access_allowed {
        return false;
    }
    if crate::arch::page_is_mapped(page_address) {
        return true;
    }

    let Some(frame) = crate::physical::allocate() else {
        return false;
    };
    let physical_address = frame.address();
    if !crate::arch::set_page_permissions(physical_address, PAGE_SIZE, 2) {
        let _released = crate::physical::deallocate(frame);
        return false;
    }
    unsafe { core::ptr::write_bytes(physical_address as *mut u8, 0, PAGE_SIZE) };
    if crate::arch::map_page(page_address, physical_address, permissions) {
        return true;
    }
    let _released = crate::physical::deallocate(frame);
    false
}

pub(crate) fn is_identity_mapped(address: usize) -> bool {
    crate::arch::is_identity_mapped(address)
}

pub(crate) fn translate(address: usize) -> Option<usize> {
    is_identity_mapped(address).then_some(address)
}
