/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ptr::{addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::kernel::scheduler::sync::IrqSpinLock;

const PAGE_SIZE: usize = 4096;
const PAGE_TABLE_ENTRIES: usize = 512;
const PAGE_TABLE_COUNT: usize = 2048;
const PRESENT: u64 = 1;
const WRITABLE: u64 = 1 << 1;
const HUGE_PAGE: u64 = 1 << 7;
const HUGE_PAGE_PAT: u64 = 1 << 12;
const PAGE_PAT: u64 = 1 << 7;
const NO_EXECUTE: u64 = 1 << 63;
const PAGE_ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const HUGE_ADDRESS_MASK: u64 = 0x000f_ffff_ffe0_0000;
const GIGA_ADDRESS_MASK: u64 = 0x000f_ffff_c000_0000;
const PAGE_FLAGS_MASK: u64 = 0x0000_0000_0000_0fff;

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
struct PageTable {
    entries: [u64; PAGE_TABLE_ENTRIES],
}

static mut PAGE_TABLES: [PageTable; PAGE_TABLE_COUNT] = [PageTable {
    entries: [0; PAGE_TABLE_ENTRIES],
}; PAGE_TABLE_COUNT];
static NEXT_PAGE_TABLE: AtomicUsize = AtomicUsize::new(0);
static NX_ENABLED: AtomicBool = AtomicBool::new(false);
static PAGE_TABLE_LOCK: IrqSpinLock<()> = IrqSpinLock::new(());

extern "C" {
    static mut pml4: u64;
}

pub(crate) fn initialize() {
    let cr0: u64;
    unsafe {
        let features = core::arch::x86_64::__cpuid(0x8000_0001);
        if features.edx & (1 << 20) != 0 {
            let low: u32;
            let high: u32;
            core::arch::asm!(
                "rdmsr",
                in("ecx") 0xc000_0080u32,
                out("eax") low,
                out("edx") high,
                options(nostack, preserves_flags),
            );
            let efer = (((high as u64) << 32) | low as u64) | (1 << 11);
            core::arch::asm!(
                "wrmsr",
                in("ecx") 0xc000_0080u32,
                in("eax") efer as u32,
                in("edx") (efer >> 32) as u32,
                options(nostack, preserves_flags),
            );
            NX_ENABLED.store(true, Ordering::Release);
        }
        core::arch::asm!(
            "mov {value}, cr0",
            value = out(reg) cr0,
            options(nomem, nostack, preserves_flags),
        );
    }
    assert!(cr0 & (1 << 31) != 0, "x86 paging was not enabled by boot");
}

pub(crate) fn set_page_permissions(address: usize, length: usize, permissions: i32) -> bool {
    if length == 0
        || address % PAGE_SIZE != 0
        || length % PAGE_SIZE != 0
        || address.checked_add(length).is_none()
    {
        return false;
    }
    if matches!(permissions, 1 | 2) && !NX_ENABLED.load(Ordering::Acquire) {
        return false;
    }

    let Some(flags) = permission_flags(permissions) else {
        return false;
    };

    let _guard = PAGE_TABLE_LOCK.lock();
    for offset in (0..length).step_by(PAGE_SIZE) {
        let Some(entry) = page_table_entry(address + offset, false) else {
            return false;
        };
        let current = unsafe { read_volatile(entry) };
        let address_bits = current & PAGE_ADDRESS_MASK;
        let preserved_flags = current & PAGE_FLAGS_MASK & !(PRESENT | WRITABLE | NO_EXECUTE);
        unsafe {
            write_volatile(entry, address_bits | preserved_flags | flags);
            core::arch::asm!(
                "invlpg [{}]",
                in(reg) (address + offset),
                options(nostack, preserves_flags),
            );
        }
    }
    true
}

pub(crate) fn map_page(virtual_address: usize, physical_address: usize, permissions: i32) -> bool {
    if virtual_address % PAGE_SIZE != 0 || physical_address % PAGE_SIZE != 0 {
        return false;
    }
    let Some(physical_end) = physical_address.checked_add(PAGE_SIZE) else {
        return false;
    };
    if physical_end > crate::kernel::memory::resources::MAX_PHYSICAL_ADDRESS
        || (matches!(permissions, 1 | 2) && !NX_ENABLED.load(Ordering::Acquire))
    {
        return false;
    }
    let Some(flags) = permission_flags(permissions) else {
        return false;
    };
    if flags & PRESENT == 0 {
        return false;
    }

    let _guard = PAGE_TABLE_LOCK.lock();
    let Some(entry) = page_table_entry(virtual_address, true) else {
        return false;
    };
    if unsafe { read_volatile(entry) } & PRESENT != 0 {
        return false;
    }
    unsafe {
        write_volatile(entry, physical_address as u64 | flags);
        core::arch::asm!(
            "invlpg [{}]",
            in(reg) virtual_address,
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
    let entry = page_table_entry(virtual_address, false)?;
    let current = unsafe { read_volatile(entry) };
    if current == 0 {
        return None;
    }
    unsafe {
        write_volatile(entry, 0);
        core::arch::asm!(
            "invlpg [{}]",
            in(reg) virtual_address,
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
    match page_table_entry(virtual_address, false) {
        Some(entry) => unsafe { read_volatile(entry) & PRESENT != 0 },
        None => false,
    }
}

pub(crate) fn page_has_backing(virtual_address: usize) -> bool {
    if virtual_address % PAGE_SIZE != 0 {
        return false;
    }
    let _guard = PAGE_TABLE_LOCK.lock();
    match page_table_entry(virtual_address, false) {
        Some(entry) => unsafe { read_volatile(entry) & PAGE_ADDRESS_MASK != 0 },
        None => false,
    }
}

fn permission_flags(permissions: i32) -> Option<u64> {
    match permissions {
        0 | 5 => Some(0),
        1 => Some(PRESENT | NO_EXECUTE),
        2 => Some(PRESENT | WRITABLE | NO_EXECUTE),
        3 => Some(PRESENT | WRITABLE),
        4 => Some(PRESENT),
        _ => None,
    }
}

fn page_table_entry(address: usize, create: bool) -> Option<*mut u64> {
    let upper_bits = address >> 48;
    let sign_bit = (address >> 47) & 1;
    if (sign_bit == 0 && upper_bits != 0) || (sign_bit != 0 && upper_bits != 0xffff) {
        return None;
    }

    let pml4_index = (address >> 39) & 0x1ff;
    let pdpt_index = (address >> 30) & 0x1ff;
    let directory_index = (address >> 21) & 0x1ff;
    let page_index = (address >> 12) & 0x1ff;

    let pml4_table = addr_of_mut!(pml4).cast::<u64>();
    let pdpt = unsafe { next_table(pml4_table.add(pml4_index), create, 3)? };
    let directory = unsafe { next_table(pdpt.add(pdpt_index), create, 2)? };
    let page_table = unsafe { next_table(directory.add(directory_index), create, 1)? };
    Some(unsafe { page_table.add(page_index) })
}

unsafe fn allocate_page_table() -> Option<*mut u64> {
    let table_index = NEXT_PAGE_TABLE.fetch_add(1, Ordering::AcqRel);
    if table_index >= PAGE_TABLE_COUNT {
        return None;
    }
    let table = unsafe {
        addr_of_mut!(PAGE_TABLES)
            .cast::<PageTable>()
            .add(table_index)
    };
    let entries = unsafe { addr_of_mut!((*table).entries).cast::<u64>() };
    for index in 0..PAGE_TABLE_ENTRIES {
        unsafe { write_volatile(entries.add(index), 0) };
    }
    Some(entries)
}

unsafe fn next_table(entry: *mut u64, create: bool, level: usize) -> Option<*mut u64> {
    let value = unsafe { read_volatile(entry) };
    if value & PRESENT == 0 {
        if !create {
            return None;
        }
        let table = unsafe { allocate_page_table()? };
        unsafe {
            write_volatile(
                entry,
                (table as u64 & PAGE_ADDRESS_MASK) | PRESENT | WRITABLE,
            );
        }
        return Some(table);
    }

    if value & HUGE_PAGE != 0 {
        if !matches!(level, 1 | 2) {
            return None;
        }
        let table = unsafe { allocate_page_table()? };
        if level == 2 {
            let physical_base = value & GIGA_ADDRESS_MASK;
            let page_flags = value & (PAGE_FLAGS_MASK | HUGE_PAGE_PAT | NO_EXECUTE);
            for index in 0..PAGE_TABLE_ENTRIES {
                let physical_address = physical_base + ((index as u64) << 21);
                unsafe { write_volatile(table.add(index), physical_address | page_flags) };
            }
        } else {
            let physical_base = value & HUGE_ADDRESS_MASK;
            let mut page_flags = value & (PAGE_FLAGS_MASK | NO_EXECUTE) & !HUGE_PAGE;
            if value & HUGE_PAGE_PAT != 0 {
                page_flags |= PAGE_PAT;
            }
            for index in 0..PAGE_TABLE_ENTRIES {
                let physical_address = physical_base + (index * PAGE_SIZE) as u64;
                unsafe { write_volatile(table.add(index), physical_address | page_flags) };
            }
        }
        unsafe {
            write_volatile(
                entry,
                (table as u64 & PAGE_ADDRESS_MASK) | PRESENT | WRITABLE,
            );
        }
        return Some(table);
    }

    Some((value & PAGE_ADDRESS_MASK) as *mut u64)
}
