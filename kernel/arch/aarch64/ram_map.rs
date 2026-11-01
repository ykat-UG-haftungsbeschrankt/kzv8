use core::ptr::{addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::sync::IrqSpinLock;

const TABLE_ENTRIES: usize = 512;
const BLOCK_SIZE: u64 = 1 << 30;
const BLOCK_DESCRIPTOR: u64 = 0b01;
const ACCESS_FLAG: u64 = 1 << 10;
const INNER_SHAREABLE: u64 = 0b11 << 8;
const UXN: u64 = 1 << 54;
const PXN: u64 = 1 << 53;
const PAGE_SIZE: usize = 4096;
const L2_BLOCK_SIZE: u64 = 1 << 21;
const PAGE_TABLE_COUNT: usize = 2048;
const L1_ADDRESS_MASK: u64 = 0x0000_ffff_c000_0000;
const L2_ADDRESS_MASK: u64 = 0x0000_ffff_ffe0_0000;
const PAGE_ADDRESS_MASK: u64 = 0x0000_ffff_ffff_f000;
const AP_MASK: u64 = 0b11 << 6;
const AP_READ_ONLY: u64 = 0b10 << 6;
const RAM_ATTRIBUTES: u64 = BLOCK_DESCRIPTOR | ACCESS_FLAG | INNER_SHAREABLE | UXN;
const TABLE_ADDRESS_MASK: u64 = 0x0000_ffff_ffff_f000;

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
struct PageTable([u64; TABLE_ENTRIES]);

static mut PAGE_TABLES: [PageTable; PAGE_TABLE_COUNT] =
    [PageTable([0; TABLE_ENTRIES]); PAGE_TABLE_COUNT];
static NEXT_PAGE_TABLE: AtomicUsize = AtomicUsize::new(0);
static PAGE_TABLE_LOCK: IrqSpinLock<()> = IrqSpinLock::new(());

pub(crate) unsafe fn map_memory_regions(regions: &[crate::platform::MemoryRegion]) {
    let root: u64;
    unsafe {
        core::arch::asm!(
            "mrs {root}, ttbr0_el1",
            root = out(reg) root,
            options(nomem, nostack, preserves_flags),
        );
    }
    let table = (root & TABLE_ADDRESS_MASK) as *mut u64;

    for region in regions {
        let Some(end) = region.start.checked_add(region.length) else {
            continue;
        };
        if region.length == 0 {
            continue;
        }
        let mut block = (region.start as u64) & !(BLOCK_SIZE - 1);
        let aligned_end = (end as u64).saturating_add(BLOCK_SIZE - 1) & !(BLOCK_SIZE - 1);
        while block < aligned_end {
            let index = (block / BLOCK_SIZE) as usize;
            if index != 0 && index < TABLE_ENTRIES {
                unsafe { write_volatile(table.add(index), block | RAM_ATTRIBUTES) };
            }
            block += BLOCK_SIZE;
        }
    }

    unsafe {
        core::arch::asm!(
            "dsb ishst",
            "tlbi vmalle1",
            "dsb ish",
            "isb",
            options(nostack, preserves_flags),
        );
    }
}

unsafe fn allocate_page_table() -> Option<*mut u64> {
    let index = NEXT_PAGE_TABLE.fetch_add(1, Ordering::AcqRel);
    if index >= PAGE_TABLE_COUNT {
        return None;
    }
    let table = unsafe { addr_of_mut!(PAGE_TABLES).cast::<PageTable>().add(index) };
    let entries = unsafe { addr_of_mut!((*table).0).cast::<u64>() };
    for entry in 0..TABLE_ENTRIES {
        unsafe { write_volatile(entries.add(entry), 0) };
    }
    Some(entries)
}

unsafe fn split_l2_block(entry: *mut u64, create: bool) -> Option<*mut u64> {
    let descriptor = unsafe { read_volatile(entry) };
    match descriptor & 0b11 {
        0 if create => {
            let table = unsafe { allocate_page_table()? };
            unsafe { write_volatile(entry, (table as u64 & TABLE_ADDRESS_MASK) | 0b11) };
            Some(table)
        }
        0 => None,
        0b11 => Some((descriptor & TABLE_ADDRESS_MASK) as *mut u64),
        0b01 => {
            let table = unsafe { allocate_page_table()? };
            let base = descriptor & L1_ADDRESS_MASK;
            let attributes = descriptor & !L1_ADDRESS_MASK & !0b11;
            for index in 0..TABLE_ENTRIES {
                let address = base + (index as u64 * L2_BLOCK_SIZE);
                unsafe {
                    write_volatile(table.add(index), address | attributes | BLOCK_DESCRIPTOR);
                }
            }
            unsafe { write_volatile(entry, (table as u64 & TABLE_ADDRESS_MASK) | 0b11) };
            Some(table)
        }
        _ => None,
    }
}

unsafe fn split_l1_block(entry: *mut u64, create: bool) -> Option<*mut u64> {
    let descriptor = unsafe { read_volatile(entry) };
    match descriptor & 0b11 {
        0 if create => {
            let table = unsafe { allocate_page_table()? };
            unsafe { write_volatile(entry, (table as u64 & TABLE_ADDRESS_MASK) | 0b11) };
            Some(table)
        }
        0 => None,
        0b11 => Some((descriptor & TABLE_ADDRESS_MASK) as *mut u64),
        0b01 => {
            let table = unsafe { allocate_page_table()? };
            let base = descriptor & L2_ADDRESS_MASK;
            let attributes = descriptor & !L2_ADDRESS_MASK & !0b11;
            for index in 0..TABLE_ENTRIES {
                let address = base + (index as u64 * PAGE_SIZE as u64);
                unsafe {
                    write_volatile(table.add(index), address | attributes | 0b11);
                }
            }
            unsafe { write_volatile(entry, (table as u64 & TABLE_ADDRESS_MASK) | 0b11) };
            Some(table)
        }
        _ => None,
    }
}

unsafe fn page_table_entry(address: usize, create: bool) -> Option<*mut u64> {
    if address >> 39 != 0 {
        return None;
    }
    let root: u64;
    unsafe {
        core::arch::asm!(
            "mrs {root}, ttbr0_el1",
            root = out(reg) root,
            options(nomem, nostack, preserves_flags),
        );
    }
    let root_table = (root & TABLE_ADDRESS_MASK) as *mut u64;
    let l1_entry = unsafe { root_table.add((address >> 30) & 0x1ff) };
    let l2_table = unsafe { split_l1_block(l1_entry, create)? };
    let l2_entry = unsafe { l2_table.add((address >> 21) & 0x1ff) };
    let l3_table = unsafe { split_l2_block(l2_entry, create)? };
    Some(unsafe { l3_table.add((address >> 12) & 0x1ff) })
}

fn page_attributes(permissions: i32) -> Option<u64> {
    if !matches!(permissions, 1..=4) {
        return None;
    }
    let read_only = matches!(permissions, 1 | 4);
    let executable = matches!(permissions, 3 | 4);
    let mut attributes = ACCESS_FLAG | INNER_SHAREABLE | UXN;
    if read_only {
        attributes |= AP_READ_ONLY;
    }
    if !executable {
        attributes |= PXN;
    }
    Some(attributes)
}

pub(crate) fn map_page(
    virtual_address: usize,
    physical_address: usize,
    permissions: i32,
) -> bool {
    if virtual_address % PAGE_SIZE != 0 || physical_address % PAGE_SIZE != 0 {
        return false;
    }
    let Some(physical_end) = physical_address.checked_add(PAGE_SIZE) else {
        return false;
    };
    if physical_end > crate::platform::MAX_PHYSICAL_ADDRESS {
        return false;
    }
    let Some(attributes) = page_attributes(permissions) else {
        return false;
    };

    let _guard = PAGE_TABLE_LOCK.lock();
    let Some(entry) = (unsafe { page_table_entry(virtual_address, true) }) else {
        return false;
    };
    if unsafe { read_volatile(entry) } & 0b11 != 0 {
        return false;
    }
    unsafe {
        write_volatile(entry, physical_address as u64 | attributes | 0b11);
        core::arch::asm!(
            "dsb ishst",
            "tlbi vaae1is, {address}",
            "dsb ish",
            "isb",
            address = in(reg) (virtual_address >> 12),
            options(nostack, preserves_flags),
        );
    }
    true
}

pub(crate) fn unmap_page(virtual_address: usize) -> Option<usize> {
    if virtual_address % PAGE_SIZE != 0 {
        return None;
    }
    let _guard = PAGE_TABLE_LOCK.lock();
    let entry = unsafe { page_table_entry(virtual_address, false)? };
    let current = unsafe { read_volatile(entry) };
    if current & 0b11 != 0b11 {
        return None;
    }
    unsafe {
        write_volatile(entry, 0);
        core::arch::asm!(
            "dsb ishst",
            "tlbi vaae1is, {address}",
            "dsb ish",
            "isb",
            address = in(reg) (virtual_address >> 12),
            options(nostack, preserves_flags),
        );
    }
    Some((current & PAGE_ADDRESS_MASK) as usize)
}

pub(crate) fn page_is_mapped(virtual_address: usize) -> bool {
    if virtual_address % PAGE_SIZE != 0 {
        return false;
    }
    let _guard = PAGE_TABLE_LOCK.lock();
    let Some(entry) = (unsafe { page_table_entry(virtual_address, false) }) else {
        return false;
    };
    unsafe { read_volatile(entry) & 0b11 == 0b11 }
}

pub(crate) fn set_page_permissions(address: usize, length: usize, permissions: i32) -> bool {
    if length == 0
        || address % PAGE_SIZE != 0
        || length % PAGE_SIZE != 0
        || address.checked_add(length).is_none()
        || address.checked_add(length).unwrap() > crate::platform::MAX_PHYSICAL_ADDRESS
    {
        return false;
    }
    if !matches!(permissions, 0..=4 | 5) {
        return false;
    }

    let _guard = PAGE_TABLE_LOCK.lock();
    for page in (address..address + length).step_by(PAGE_SIZE) {
        let Some(entry) = (unsafe { page_table_entry(page, false) }) else {
            return false;
        };
        let current = unsafe { read_volatile(entry) };
        if matches!(permissions, 0 | 5) {
            unsafe { write_volatile(entry, 0) };
        } else {
            let physical_address = if current & 0b11 == 0 {
                page as u64 & PAGE_ADDRESS_MASK
            } else {
                current & PAGE_ADDRESS_MASK
            };
            let Some(page_attributes) = page_attributes(permissions) else {
                return false;
            };
            let attributes =
                (current & !(PAGE_ADDRESS_MASK | AP_MASK | PXN | UXN | 0b11)) | page_attributes;
            unsafe { write_volatile(entry, physical_address | attributes | 0b11) };
        }
        unsafe {
            core::arch::asm!(
                "dsb ishst",
                "tlbi vaae1is, {address}",
                "dsb ish",
                "isb",
                address = in(reg) (page >> 12),
                options(nostack, preserves_flags),
            );
        }
    }
    true
}
