use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicU8, Ordering};

use crate::sync::IrqSpinLock;

pub(crate) const PAGE_SIZE: usize = 4096;

const VM_ARENA_SIZE: usize = 1 << 30;
const MAX_VM_PAGES: usize = VM_ARENA_SIZE / PAGE_SIZE;
const MAX_VM_AREAS: usize = 16384;
const VM_RESERVED: u8 = 1 << 7;
const VM_PERMISSION_MASK: u8 = 0x07;

#[cfg(target_arch = "x86_64")]
const VM_BASE: usize = 0xffff_8000_0000_0000;
#[cfg(target_arch = "aarch64")]
const VM_BASE: usize = 511usize << 30;

static PAGE_STATE: [AtomicU8; MAX_VM_PAGES] = [const { AtomicU8::new(0) }; MAX_VM_PAGES];
static VM_LOCK: IrqSpinLock<()> = IrqSpinLock::new(());

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

static mut VM_AREAS: [VmArea; MAX_VM_AREAS] = [VmArea::EMPTY; MAX_VM_AREAS];

#[derive(Clone, Copy)]
pub(crate) enum FaultAccess {
    Read,
    Write,
    Execute,
}

pub(super) fn initialize() {
    crate::arch::initialize_mmu();
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
    if length == 0
        || alignment == 0
        || !alignment.is_power_of_two()
        || !(0..=4).contains(&permissions)
    {
        return None;
    }
    let length = rounded_length(length)?;
    let page_count = length / PAGE_SIZE;
    if page_count > MAX_VM_PAGES {
        return None;
    }
    let alignment = alignment.max(PAGE_SIZE);

    let _guard = VM_LOCK.lock();
    let areas = addr_of_mut!(VM_AREAS).cast::<VmArea>();
    let mut area_slot = None;
    for index in 0..MAX_VM_AREAS {
        if !unsafe { (*areas.add(index)).active } {
            area_slot = Some(index);
            break;
        }
    }
    let area_slot = area_slot?;

    let mut candidate_page = 0usize;
    loop {
        let candidate_address = VM_BASE.checked_add(candidate_page.checked_mul(PAGE_SIZE)?)?;
        let address = align_up(candidate_address, alignment)?;
        let start_page = address.checked_sub(VM_BASE)? / PAGE_SIZE;
        let end_page = start_page.checked_add(page_count)?;
        if end_page > MAX_VM_PAGES {
            return None;
        }

        let occupied = (start_page..end_page)
            .find(|page| PAGE_STATE[*page].load(Ordering::Acquire) & VM_RESERVED != 0);
        if let Some(occupied_page) = occupied {
            candidate_page = occupied_page + 1;
            continue;
        }

        let state = VM_RESERVED | permissions as u8;
        for page in start_page..end_page {
            PAGE_STATE[page].store(state, Ordering::Release);
        }
        unsafe {
            areas.add(area_slot).write(VmArea {
                start_page,
                page_count,
                active: true,
            });
        }
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

    let _guard = VM_LOCK.lock();
    let areas = addr_of_mut!(VM_AREAS).cast::<VmArea>();
    let mut area_slot = None;
    for index in 0..MAX_VM_AREAS {
        let area = unsafe { &*areas.add(index) };
        if area.active && area.start_page == start_page && area.page_count == page_count {
            area_slot = Some(index);
            break;
        }
    }
    let Some(area_slot) = area_slot else {
        return false;
    };
    if (start_page..end_page)
        .any(|page| PAGE_STATE[page].load(Ordering::Acquire) & VM_RESERVED == 0)
    {
        return false;
    }

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
    unsafe { areas.add(area_slot).write(VmArea::EMPTY) };
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

    let _guard = VM_LOCK.lock();
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
    #[cfg(target_arch = "x86_64")]
    {
        address < crate::platform::MAX_PHYSICAL_ADDRESS
    }
    #[cfg(target_arch = "aarch64")]
    {
        address < 0x4000_0000
            || crate::platform::memory_regions().iter().any(|region| {
                region
                    .start
                    .checked_add(region.length)
                    .is_some_and(|end| (region.start..end).contains(&address))
            })
    }
}

pub(crate) fn translate(address: usize) -> Option<usize> {
    is_identity_mapped(address).then_some(address)
}
