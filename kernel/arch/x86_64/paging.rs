/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ptr::{addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::kernel::arch::traits::MemoryArchitecture as _;
use crate::kernel::scheduler::sync::IrqSpinLock;

const PAGE_SIZE: usize = 4096;
const PAGE_TABLE_ENTRIES: usize = 512;
const PAGE_TABLE_COUNT: usize = 2048;
const PRESENT: u64 = 1;
const WRITABLE: u64 = 1 << 1;
const USER_ACCESS: u64 = 1 << 2;
const WRITE_THROUGH: u64 = 1 << 3;
const CACHE_DISABLE: u64 = 1 << 4;
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

    let _guard = PAGE_TABLE_LOCK.lock();
    for offset in (0..length).step_by(PAGE_SIZE) {
        let Some(entry) = page_table_entry(address + offset, false, user) else {
            return false;
        };
        let current = unsafe { read_volatile(entry) };
        let user_access = user || current & USER_ACCESS != 0;
        let Some(flags) = permission_flags(permissions, user_access) else {
            return false;
        };
        let address_bits = current & PAGE_ADDRESS_MASK;
        let preserved_flags =
            current & PAGE_FLAGS_MASK & !(PRESENT | WRITABLE | USER_ACCESS | NO_EXECUTE);
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
    map_page_inner(virtual_address, physical_address, permissions, false)
}

pub(crate) fn read_mmio_u32(physical_address: usize) -> Option<u32> {
    if physical_address % core::mem::size_of::<u32>() != 0 {
        return None;
    }
    with_uncached_mmio(physical_address, |address| unsafe {
        read_volatile(address.cast::<u32>())
    })
}

fn with_uncached_mmio<T>(physical_address: usize, read: impl FnOnce(*const u8) -> T) -> Option<T> {
    let page_address = physical_address & !(PAGE_SIZE - 1);
    let page_end = page_address.checked_add(PAGE_SIZE)?;
    let physical_limit = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::physical_address_limit() as usize;
    if page_end > physical_limit
        || !crate::kernel::arch::MemoryArchitecture::is_identity_mapped(page_address)
        || !crate::kernel::arch::MemoryArchitecture::is_identity_mapped(page_end - 1)
    {
        return None;
    }
    let memory_regions = crate::kernel::memory::memblock::MemBlock::memory_regions();
    if memory_regions.iter().any(|region| {
        region
            .start
            .checked_add(region.length)
            .is_some_and(|end| region.start < page_end && page_address < end)
    }) {
        return None;
    }

    let _guard = PAGE_TABLE_LOCK.lock();
    let entry = page_table_entry(page_address, false, false)?;
    let original = unsafe { read_volatile(entry) };
    if original & PRESENT == 0 {
        return None;
    }
    let cache_flags = WRITE_THROUGH | CACHE_DISABLE | PAGE_PAT;
    unsafe {
        write_volatile(
            entry,
            (original & !cache_flags) | WRITE_THROUGH | CACHE_DISABLE,
        );
        core::arch::asm!(
            "invlpg [{}]",
            in(reg) page_address,
            options(nostack, preserves_flags),
        );
    }
    let value = read(physical_address as *const u8);
    unsafe {
        write_volatile(entry, original);
        core::arch::asm!(
            "invlpg [{}]",
            in(reg) page_address,
            options(nostack, preserves_flags),
        );
    }
    Some(value)
}

pub(crate) fn read_mmio_u64(physical_address: usize) -> Option<u64> {
    if physical_address % core::mem::size_of::<u64>() != 0 {
        return None;
    }
    with_uncached_mmio(physical_address, |address| unsafe {
        read_volatile(address.cast::<u64>())
    })
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
        || (matches!(permissions, 1 | 2) && !NX_ENABLED.load(Ordering::Acquire))
    {
        return false;
    }
    let Some(flags) = permission_flags(permissions, user) else {
        return false;
    };
    if flags & PRESENT == 0 {
        return false;
    }

    let _guard = PAGE_TABLE_LOCK.lock();
    let Some(entry) = page_table_entry(virtual_address, true, user) else {
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
    let entry = page_table_entry(virtual_address, false, false)?;
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
    match page_table_entry(virtual_address, false, false) {
        Some(entry) => unsafe { read_volatile(entry) & PRESENT != 0 },
        None => false,
    }
}

pub(crate) fn page_has_backing(virtual_address: usize) -> bool {
    if virtual_address % PAGE_SIZE != 0 {
        return false;
    }
    let _guard = PAGE_TABLE_LOCK.lock();
    match page_table_entry(virtual_address, false, false) {
        Some(entry) => unsafe { read_volatile(entry) & PAGE_ADDRESS_MASK != 0 },
        None => false,
    }
}

pub(crate) fn user_range_accessible(address: usize, length: usize, write: bool) -> bool {
    const USER_ADDRESS_LIMIT: usize = 1 << 47;
    if length == 0 {
        return address <= USER_ADDRESS_LIMIT;
    }
    let Some(end) = address.checked_add(length) else {
        return false;
    };
    if end > USER_ADDRESS_LIMIT {
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
        let Some(entry) = page_table_entry(page, false, false) else {
            return false;
        };
        let flags = unsafe { read_volatile(entry) };
        if flags & (PRESENT | USER_ACCESS) != (PRESENT | USER_ACCESS)
            || (write && flags & WRITABLE == 0)
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
    const USER_ADDRESS_LIMIT: usize = 1 << 47;
    if length == 0 {
        return address <= USER_ADDRESS_LIMIT;
    }
    let Some(end) = address.checked_add(length) else {
        return false;
    };
    if end > USER_ADDRESS_LIMIT {
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
        let Some(entry) = page_table_entry(page, false, false) else {
            return false;
        };
        let flags = unsafe { read_volatile(entry) };
        if flags & (PRESENT | USER_ACCESS) != (PRESENT | USER_ACCESS) || flags & NO_EXECUTE != 0 {
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

fn permission_flags(permissions: i32, user: bool) -> Option<u64> {
    let flags = match permissions {
        0 | 5 => Some(0),
        1 => Some(PRESENT | NO_EXECUTE),
        2 => Some(PRESENT | WRITABLE | NO_EXECUTE),
        3 => Some(PRESENT | WRITABLE),
        4 => Some(PRESENT),
        _ => None,
    }?;
    Some(flags | if user && flags & PRESENT != 0 { USER_ACCESS } else { 0 })
}

fn page_table_entry(address: usize, create: bool, user: bool) -> Option<*mut u64> {
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
    let pdpt = unsafe { next_table(pml4_table.add(pml4_index), create, 3, user)? };
    let directory = unsafe { next_table(pdpt.add(pdpt_index), create, 2, user)? };
    let page_table = unsafe { next_table(directory.add(directory_index), create, 1, user)? };
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

unsafe fn next_table(
    entry: *mut u64,
    create: bool,
    level: usize,
    user: bool,
) -> Option<*mut u64> {
    let value = unsafe { read_volatile(entry) };
    if value & PRESENT == 0 {
        if !create {
            return None;
        }
        let table = unsafe { allocate_page_table()? };
        let user_flag = if user { USER_ACCESS } else { 0 };
        unsafe {
            write_volatile(
                entry,
                (table as u64 & PAGE_ADDRESS_MASK) | PRESENT | WRITABLE | user_flag,
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
        let user_flag = if user { USER_ACCESS } else { 0 };
        unsafe {
            write_volatile(
                entry,
                (table as u64 & PAGE_ADDRESS_MASK) | PRESENT | WRITABLE | user_flag,
            );
        }
        return Some(table);
    }

    if user && value & USER_ACCESS == 0 {
        unsafe { write_volatile(entry, value | USER_ACCESS) };
    }

    Some((value & PAGE_ADDRESS_MASK) as *mut u64)
}
