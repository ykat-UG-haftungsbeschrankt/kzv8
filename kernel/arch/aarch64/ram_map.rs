/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ptr::{addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::kernel::scheduler::sync::IrqSpinLock;

const TABLE_ENTRIES: usize = 512;
const BLOCK_SIZE: u64 = 1 << 30;
pub(crate) const IDENTITY_MAP_LIMIT: usize = TABLE_ENTRIES * BLOCK_SIZE as usize;
const BLOCK_DESCRIPTOR: u64 = 0b01;
const ACCESS_FLAG: u64 = 1 << 10;
const PAGE_DESCRIPTOR: u64 = 0b11;
const INNER_SHAREABLE: u64 = 0b11 << 8;
const UXN: u64 = 1 << 54;
const PXN: u64 = 1 << 53;
const PAGE_SIZE: usize = 4096;
const L2_BLOCK_SIZE: u64 = 1 << 21;
const PAGE_TABLE_COUNT: usize = 2048;
const VIRTUAL_ADDRESS_LIMIT: usize = 1usize << 39;
const L1_ADDRESS_MASK: u64 = 0x0000_ffff_c000_0000;
const L2_ADDRESS_MASK: u64 = 0x0000_ffff_ffe0_0000;
const PAGE_ADDRESS_MASK: u64 = 0x0000_ffff_ffff_f000;
const AP_MASK: u64 = 0b11 << 6;
const AP_READ_ONLY: u64 = 0b10 << 6;
const AP_USER_READ_WRITE: u64 = 0b01 << 6;
const RAM_ATTRIBUTES: u64 = BLOCK_DESCRIPTOR | ACCESS_FLAG | INNER_SHAREABLE | UXN;
const TABLE_ADDRESS_MASK: u64 = 0x0000_ffff_ffff_f000;

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
struct PageTable([u64; TABLE_ENTRIES]);

static mut PAGE_TABLES: [PageTable; PAGE_TABLE_COUNT] =
    [PageTable([0; TABLE_ENTRIES]); PAGE_TABLE_COUNT];
static NEXT_PAGE_TABLE: AtomicUsize = AtomicUsize::new(0);
static PAGE_TABLE_FREE: [AtomicBool; PAGE_TABLE_COUNT] =
    [const { AtomicBool::new(false) }; PAGE_TABLE_COUNT];
static PAGE_TABLE_LOCK: IrqSpinLock<()> = IrqSpinLock::new(());

pub(crate) unsafe fn map_memory_regions(
    regions: &[crate::kernel::memory::memblock::MemoryRegion],
) {
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
        let end = end.min(IDENTITY_MAP_LIMIT);
        if region.start >= end {
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
    let used_tables = NEXT_PAGE_TABLE.load(Ordering::Acquire).min(PAGE_TABLE_COUNT);
    let reusable = (0..used_tables).find(|index| {
        PAGE_TABLE_FREE[*index].swap(false, Ordering::AcqRel)
    });
    let index = reusable.unwrap_or_else(|| NEXT_PAGE_TABLE.fetch_add(1, Ordering::AcqRel));
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

fn free_page_table(table: *mut u64) -> bool {
    let tables_start = unsafe { addr_of_mut!(PAGE_TABLES).cast::<PageTable>() as usize };
    let Some(offset) = (table as usize).checked_sub(tables_start) else {
        return false;
    };
    if offset % PAGE_SIZE != 0 {
        return false;
    }
    let index = offset / PAGE_SIZE;
    if index >= PAGE_TABLE_COUNT
        || index >= NEXT_PAGE_TABLE.load(Ordering::Acquire)
        || PAGE_TABLE_FREE[index].swap(true, Ordering::AcqRel)
    {
        return false;
    }
    true
}

pub(crate) fn create_address_space() -> Option<(usize, usize, usize)> {
    const KERNEL_VM_START: usize = 448;
    const USER_VM_MAX_ENTRIES: usize = 64;

    let _guard = PAGE_TABLE_LOCK.lock();
    let kernel_root = addr_of_mut!(TTBR0_TABLE.0).cast::<u64>();
    for index in KERNEL_VM_START..TABLE_ENTRIES {
        unsafe { split_l1_block(kernel_root.add(index), true)? };
    }

    let mut best_start = 0;
    let mut best_length = 0;
    let mut run_start = 0;
    let mut run_length = 0;
    for index in 2..KERNEL_VM_START {
        if unsafe { read_volatile(kernel_root.add(index)) } == 0 {
            if run_length == 0 {
                run_start = index;
            }
            run_length += 1;
        } else {
            if run_length > best_length {
                best_start = run_start;
                best_length = run_length;
            }
            run_length = 0;
        }
    }
    if run_length > best_length {
        best_start = run_start;
        best_length = run_length;
    }
    let arena_entries = best_length.min(USER_VM_MAX_ENTRIES);
    if arena_entries == 0 {
        return None;
    }

    let root = unsafe { allocate_page_table()? };
    for index in 0..TABLE_ENTRIES {
        let entry = unsafe { read_volatile(kernel_root.add(index)) };
        unsafe { write_volatile(root.add(index), entry) };
    }
    Some((
        root as usize,
        best_start * BLOCK_SIZE as usize,
        arena_entries * BLOCK_SIZE as usize,
    ))
}

pub(crate) fn activate_address_space(root: usize) -> bool {
    if root == 0
        || root % PAGE_SIZE != 0
        || (root as u64 & TABLE_ADDRESS_MASK) != root as u64
    {
        return false;
    }
    unsafe {
        core::arch::asm!(
            "dsb ishst",
            "msr ttbr0_el1, {root}",
            "dsb ish",
            "tlbi vmalle1",
            "dsb ish",
            "isb",
            root = in(reg) root as u64,
            options(nostack, preserves_flags),
        );
    }
    true
}

pub(crate) fn current_address_space() -> usize {
    let root: u64;
    unsafe {
        core::arch::asm!(
            "mrs {root}, ttbr0_el1",
            root = out(reg) root,
            options(nomem, nostack, preserves_flags),
        );
    }
    (root & TABLE_ADDRESS_MASK) as usize
}

pub(crate) fn destroy_address_space(root: usize, user_base: usize, user_length: usize) -> bool {
    let Some(user_end) = user_base.checked_add(user_length) else {
        return false;
    };
    let address_limit = TABLE_ENTRIES * BLOCK_SIZE as usize;
    if root == 0
        || root % PAGE_SIZE != 0
        || (root as u64 & TABLE_ADDRESS_MASK) != root as u64
        || user_length == 0
        || user_base % BLOCK_SIZE as usize != 0
        || user_length % BLOCK_SIZE as usize != 0
        || user_end > address_limit
        || root == current_address_space()
    {
        return false;
    }

    let _guard = PAGE_TABLE_LOCK.lock();
    let root_table = root as *mut u64;
    let first_index = user_base / BLOCK_SIZE as usize;
    let end_index = user_end / BLOCK_SIZE as usize;
    for root_index in first_index..end_index {
        let root_entry = unsafe { read_volatile(root_table.add(root_index)) };
        match root_entry & 0b11 {
            0 => continue,
            0b11 => {
                let level_two = (root_entry & TABLE_ADDRESS_MASK) as *mut u64;
                for level_two_index in 0..TABLE_ENTRIES {
                    let entry = unsafe { read_volatile(level_two.add(level_two_index)) };
                    match entry & 0b11 {
                        0 => {}
                        0b11 => {
                            if !free_page_table((entry & TABLE_ADDRESS_MASK) as *mut u64) {
                                return false;
                            }
                        }
                        _ => return false,
                    }
                }
                if !free_page_table(level_two) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    free_page_table(root_table)
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
            let base = descriptor & L2_ADDRESS_MASK;
            let attributes = descriptor & !L2_ADDRESS_MASK & !0b11;
            for index in 0..TABLE_ENTRIES {
                let address = base + (index as u64 * PAGE_SIZE as u64);
                unsafe {
                    write_volatile(table.add(index), address | attributes | PAGE_DESCRIPTOR);
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
    page_attributes_for(permissions, false)
}

fn user_page_attributes(permissions: i32) -> Option<u64> {
    page_attributes_for(permissions, true)
}

fn page_attributes_for(permissions: i32, user: bool) -> Option<u64> {
    if !matches!(permissions, 1..=4) {
        return None;
    }
    let read_only = matches!(permissions, 1 | 4);
    let executable = matches!(permissions, 3 | 4);
    let mut attributes = ACCESS_FLAG | INNER_SHAREABLE | UXN;
    if read_only {
        attributes |= if user { AP_MASK } else { AP_READ_ONLY };
    } else if user {
        attributes |= AP_USER_READ_WRITE;
    }
    if user || !executable {
        attributes |= PXN;
    }
    if executable {
        attributes &= !UXN;
    }
    Some(attributes)
}

pub(crate) fn map_page(virtual_address: usize, physical_address: usize, permissions: i32) -> bool {
    map_page_inner(virtual_address, physical_address, permissions, false)
}

pub(crate) fn map_user_page(
    virtual_address: usize,
    physical_address: usize,
    permissions: i32,
) -> bool {
    map_page_inner(virtual_address, physical_address, permissions, true)
}

fn map_page_inner(
    virtual_address: usize,
    physical_address: usize,
    permissions: i32,
    user: bool,
) -> bool {
    if virtual_address % PAGE_SIZE != 0 || physical_address % PAGE_SIZE != 0 {
        return false;
    }
    let Some(physical_end) = physical_address.checked_add(PAGE_SIZE) else {
        return false;
    };
    if physical_end
        > <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::physical_address_limit() as usize
    {
        return false;
    }
    let attributes = if user {
        user_page_attributes(permissions)
    } else {
        page_attributes(permissions)
    };
    let Some(attributes) = attributes else {
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
    let descriptor_type = current & 0b11;
    let physical_address = current & PAGE_ADDRESS_MASK;
    if descriptor_type != 0b11 && (descriptor_type != 0 || physical_address == 0) {
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
    Some(physical_address as usize)
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

pub(crate) fn page_has_backing(virtual_address: usize) -> bool {
    if virtual_address % PAGE_SIZE != 0 {
        return false;
    }
    let _guard = PAGE_TABLE_LOCK.lock();
    let Some(entry) = (unsafe { page_table_entry(virtual_address, false) }) else {
        return false;
    };
    let descriptor = unsafe { read_volatile(entry) };
    descriptor & 0b11 == 0b11
        || (descriptor & 0b11 == 0 && descriptor & PAGE_ADDRESS_MASK != 0)
}

pub(crate) fn user_range_accessible(address: usize, length: usize, write: bool) -> bool {
    if length == 0 {
        return address <= VIRTUAL_ADDRESS_LIMIT;
    }
    let Some(end) = address.checked_add(length) else {
        return false;
    };
    if end > VIRTUAL_ADDRESS_LIMIT {
        return false;
    }
    let first_page = address & !(PAGE_SIZE - 1);
    let Some(last_byte) = end.checked_sub(1) else {
        return false;
    };
    let last_page = last_byte & !(PAGE_SIZE - 1);
    let _guard = PAGE_TABLE_LOCK.lock();
    let mut page = first_page;
    loop {
        let Some(entry) = (unsafe { page_table_entry(page, false) }) else {
            return false;
        };
        let descriptor = unsafe { read_volatile(entry) };
        let access_permissions = (descriptor & AP_MASK) >> 6;
        if descriptor & 0b11 != 0b11
            || descriptor & (1 << 6) == 0
            || (write && access_permissions != 0b01)
        {
            return false;
        }
        if page == last_page {
            return true;
        }
        let Some(next_page) = page.checked_add(PAGE_SIZE) else {
            return false;
        };
        page = next_page;
    }
}

pub(crate) fn user_range_executable(address: usize, length: usize) -> bool {
    if length == 0 {
        return address <= VIRTUAL_ADDRESS_LIMIT;
    }
    let Some(end) = address.checked_add(length) else {
        return false;
    };
    if end > VIRTUAL_ADDRESS_LIMIT {
        return false;
    }
    let first_page = address & !(PAGE_SIZE - 1);
    let Some(last_byte) = end.checked_sub(1) else {
        return false;
    };
    let last_page = last_byte & !(PAGE_SIZE - 1);
    let _guard = PAGE_TABLE_LOCK.lock();
    let mut page = first_page;
    loop {
        let Some(entry) = (unsafe { page_table_entry(page, false) }) else {
            return false;
        };
        let descriptor = unsafe { read_volatile(entry) };
        if descriptor & 0b11 != 0b11
            || descriptor & (1 << 6) == 0
            || descriptor & UXN != 0
        {
            return false;
        }
        if page == last_page {
            return true;
        }
        let Some(next_page) = page.checked_add(PAGE_SIZE) else {
            return false;
        };
        page = next_page;
    }
}

pub(crate) fn set_page_permissions(address: usize, length: usize, permissions: i32) -> bool {
    set_page_permissions_inner(address, length, permissions, false)
}

pub(crate) fn set_user_page_permissions(address: usize, length: usize, permissions: i32) -> bool {
    set_page_permissions_inner(address, length, permissions, true)
}

fn set_page_permissions_inner(
    address: usize,
    length: usize,
    permissions: i32,
    user: bool,
) -> bool {
    let Some(end) = address.checked_add(length) else {
        return false;
    };
    if length == 0
        || address % PAGE_SIZE != 0
        || length % PAGE_SIZE != 0
        || end > VIRTUAL_ADDRESS_LIMIT
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
            unsafe { write_volatile(entry, current & PAGE_ADDRESS_MASK) };
        } else {
            let current_address = current & PAGE_ADDRESS_MASK;
            let physical_address = if current & 0b11 == 0 && current_address == 0 {
                page as u64 & PAGE_ADDRESS_MASK
            } else {
                current_address
            };
            let page_attributes = if user {
                user_page_attributes(permissions)
            } else {
                page_attributes(permissions)
            };
            let Some(page_attributes) = page_attributes else {
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
