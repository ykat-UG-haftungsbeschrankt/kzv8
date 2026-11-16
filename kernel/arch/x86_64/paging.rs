/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ptr::{addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::kernel::arch::traits::{CpuArchitecture as _, MemoryArchitecture as _};
use crate::kernel::scheduler::sync::IrqSpinLock;

const PAGE_SIZE: usize = 4096;
const PAGE_TABLE_ENTRIES: usize = 512;
const BOOTSTRAP_PAGE_TABLE_COUNT: usize = 128;
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

static mut PAGE_TABLES: [PageTable; BOOTSTRAP_PAGE_TABLE_COUNT] = [PageTable {
    entries: [0; PAGE_TABLE_ENTRIES],
}; BOOTSTRAP_PAGE_TABLE_COUNT];
static NEXT_PAGE_TABLE: AtomicUsize = AtomicUsize::new(0);
static PAGE_TABLE_FREE: [AtomicBool; BOOTSTRAP_PAGE_TABLE_COUNT] =
    [const { AtomicBool::new(false) }; BOOTSTRAP_PAGE_TABLE_COUNT];
static NX_ENABLED: AtomicBool = AtomicBool::new(false);
static TLB_GENERATION: AtomicUsize = AtomicUsize::new(0);
static PAGE_TABLE_LOCK: IrqSpinLock<()> = IrqSpinLock::new(());

struct PageTableReservation<const N: usize> {
    frames: [usize; N],
    count: usize,
    next: usize,
    physical: bool,
}

impl<const N: usize> PageTableReservation<N> {
    fn new(virtual_address: usize) -> Self {
        let physical = crate::kernel::memory::physical::PhysicalAllocator::page_table_allocator_ready()
            && !crate::kernel::memory::physical::PhysicalAllocator::is_page_table_scratch_address(
                virtual_address,
            );
        let mut reservation = Self {
            frames: [0; N],
            count: 0,
            next: 0,
            physical,
        };
        if physical {
            while reservation.count < N {
                let Some(frame) =
                    crate::kernel::memory::physical::PhysicalAllocator::allocate_page_table_frame()
                else {
                    break;
                };
                reservation.frames[reservation.count] = frame;
                reservation.count += 1;
            }
        }
        reservation
    }

    fn allocate(&mut self) -> Option<*mut u64> {
        if !self.physical {
            return unsafe { allocate_bootstrap_page_table() };
        }
        let address = *self.frames.get(self.next)?;
        self.next += 1;
        let table = address as *mut u64;
        // SAFETY: reserved frames are page-aligned, identity-mapped, and exclusively owned.
        for index in 0..PAGE_TABLE_ENTRIES {
            unsafe { write_volatile(table.add(index), 0) };
        }
        Some(table)
    }
}

impl<const N: usize> Drop for PageTableReservation<N> {
    fn drop(&mut self) {
        if !self.physical {
            return;
        }
        for frame in &self.frames[self.next..self.count] {
            let _released = crate::kernel::memory::physical::PhysicalAllocator::deallocate_page_table_frame(*frame);
        }
    }
}

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

pub(crate) fn tlb_generation() -> usize {
    TLB_GENERATION.load(Ordering::Acquire)
}

fn flush_local_tlb() {
    let root: u64;
    unsafe {
        core::arch::asm!(
            "mov {root}, cr3",
            root = out(reg) root,
            options(nomem, nostack, preserves_flags),
        );
        core::arch::asm!(
            "mov cr3, {root}",
            root = in(reg) root,
            options(nostack, preserves_flags),
        );
    }
}

fn shootdown_remote_tlbs() {
    let previous_generation = TLB_GENERATION.fetch_update(
        Ordering::AcqRel,
        Ordering::Acquire,
        |generation| generation.checked_add(1),
    );
    let Ok(previous_generation) = previous_generation else {
        crate::kernel::arch::CpuArchitecture::halt();
    };
    let generation = previous_generation + 1;
    super::smp::acknowledge_tlb_shootdown(generation);
    if !super::smp::send_tlb_shootdown(generation) {
        crate::kernel::arch::CpuArchitecture::halt();
    }
    while !super::smp::all_online_cpus_acknowledged(generation) {
        core::hint::spin_loop();
    }
}

pub(crate) fn handle_tlb_shootdown_nmi() -> bool {
    let Some(acknowledged_generation) = super::smp::current_tlb_generation() else {
        return false;
    };
    if acknowledged_generation >= TLB_GENERATION.load(Ordering::Acquire) {
        return false;
    }
    loop {
        let generation = TLB_GENERATION.load(Ordering::Acquire);
        flush_local_tlb();
        if TLB_GENERATION.load(Ordering::Acquire) == generation {
            super::smp::acknowledge_tlb_shootdown(generation);
            if TLB_GENERATION.load(Ordering::Acquire) == generation {
                return true;
            }
        }
    }
}

pub(crate) fn create_address_space() -> Option<(usize, usize, usize)> {
    let mut tables = PageTableReservation::<1>::new(0);
    let _guard = PAGE_TABLE_LOCK.lock();
    let kernel_root = addr_of_mut!(pml4).cast::<u64>();
    let user_pml4_index = (1..256).find(|index| unsafe {
        read_volatile(kernel_root.add(*index)) == 0
    })?;
    let root = tables.allocate()?;
    // SAFETY: the reserved root and boot root are aligned, mapped page tables under the lock.
    for index in 0..PAGE_TABLE_ENTRIES {
        let entry = unsafe { read_volatile(kernel_root.add(index)) };
        unsafe { write_volatile(root.add(index), entry) };
    }
    Some((
        root as usize,
        user_pml4_index << 39,
        64usize << 30,
    ))
}

pub(crate) fn activate_address_space(root: usize) -> bool {
    if root == 0 || root % PAGE_SIZE != 0 || (root as u64 & PAGE_ADDRESS_MASK) != root as u64 {
        return false;
    }
    unsafe {
        core::arch::asm!(
            "mov cr3, {root}",
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
            "mov {root}, cr3",
            root = out(reg) root,
            options(nomem, nostack, preserves_flags),
        );
    }
    (root & PAGE_ADDRESS_MASK) as usize
}

pub(crate) fn destroy_address_space(root: usize, user_base: usize, user_length: usize) -> bool {
    let Some(user_end) = user_base.checked_add(user_length) else {
        return false;
    };
    if root == 0
        || root % PAGE_SIZE != 0
        || (root as u64 & PAGE_ADDRESS_MASK) != root as u64
        || user_length == 0
        || user_base >> 39 != (user_end - 1) >> 39
        || root == current_address_space()
    {
        return false;
    }

    let root_table = root as *mut u64;
    let pml4_index = (user_base >> 39) & 0x1ff;
    let root_entry = {
        let _guard = PAGE_TABLE_LOCK.lock();
        let root_entry = unsafe { read_volatile(root_table.add(pml4_index)) };
        if root_entry & PRESENT != 0 {
            let pdpt = (root_entry & PAGE_ADDRESS_MASK) as *mut u64;
            if root_entry & HUGE_PAGE != 0 || !unsafe { page_table_tree_is_releasable(pdpt, 3) } {
                return false;
            }
        }
        // SAFETY: root_table is validated and protected by PAGE_TABLE_LOCK.
        unsafe { write_volatile(root_table.add(pml4_index), 0) };
        root_entry
    };
    if root_entry & PRESENT != 0 {
        let pdpt = (root_entry & PAGE_ADDRESS_MASK) as *mut u64;
        if !unsafe { release_page_table_tree(pdpt, 3) } {
            return false;
        }
    }
    release_page_table(root_table)
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

    let mut tables = PageTableReservation::<2>::new(address);
    let _guard = PAGE_TABLE_LOCK.lock();
    let mut complete = true;
    let mut changed = false;
    for offset in (0..length).step_by(PAGE_SIZE) {
        let Some(entry) = page_table_entry(address + offset, false, user, &mut tables) else {
            complete = false;
            break;
        };
        let current = unsafe { read_volatile(entry) };
        let user_access = user || current & USER_ACCESS != 0;
        let address_bits = current & PAGE_ADDRESS_MASK;
        let preserved_flags =
            current & PAGE_FLAGS_MASK & !(PRESENT | WRITABLE | USER_ACCESS | NO_EXECUTE);
        let Some(flags) = permission_flags(permissions, user_access) else {
            complete = false;
            break;
        };
        unsafe {
            write_volatile(entry, address_bits | preserved_flags | flags);
            core::arch::asm!(
                "invlpg [{}]",
                in(reg) (address + offset),
                options(nostack, preserves_flags),
            );
        }
        changed = true;
    }
    if changed {
        shootdown_remote_tlbs();
    }
    complete
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

pub(crate) fn write_mmio_u32(physical_address: usize, value: u32) -> Option<()> {
    if physical_address % core::mem::size_of::<u32>() != 0 {
        return None;
    }
    with_uncached_mmio(physical_address, |address| unsafe {
        // SAFETY: with_uncached_mmio validates and uncaches this aligned MMIO register.
        write_volatile(address.cast_mut().cast::<u32>(), value);
    })
}

fn with_uncached_mmio<T>(physical_address: usize, access: impl FnOnce(*const u8) -> T) -> Option<T> {
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

    let mut tables = PageTableReservation::<2>::new(page_address);
    let _guard = PAGE_TABLE_LOCK.lock();
    let entry = page_table_entry(page_address, false, false, &mut tables)?;
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
    shootdown_remote_tlbs();
    let value = access(physical_address as *const u8);
    unsafe {
        write_volatile(entry, original);
        core::arch::asm!(
            "invlpg [{}]",
            in(reg) page_address,
            options(nostack, preserves_flags),
        );
    }
    shootdown_remote_tlbs();
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

pub(crate) fn write_mmio_u64(physical_address: usize, value: u64) -> Option<()> {
    if physical_address % core::mem::size_of::<u64>() != 0 {
        return None;
    }
    with_uncached_mmio(physical_address, |address| unsafe {
        // SAFETY: with_uncached_mmio validates and uncaches this aligned MMIO register.
        write_volatile(address.cast_mut().cast::<u64>(), value);
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

    let mut tables = PageTableReservation::<3>::new(virtual_address);
    let _guard = PAGE_TABLE_LOCK.lock();
    let Some(entry) = page_table_entry(virtual_address, true, user, &mut tables) else {
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
    shootdown_remote_tlbs();
    true
}

pub(crate) fn unmap_page(virtual_address: usize) -> Option<usize> {
    if virtual_address % PAGE_SIZE != 0 {
        return None;
    }
    let mut tables = PageTableReservation::<2>::new(virtual_address);
    let _guard = PAGE_TABLE_LOCK.lock();
    let entry = page_table_entry(virtual_address, false, false, &mut tables)?;
    let current = unsafe { read_volatile(entry) };
    if current == 0 {
        return None;
    }
    let pml4_index = (virtual_address >> 39) & 0x1ff;
    let mut reclaimed = [core::ptr::null_mut(); 3];
    let mut reclaimed_count = 0;
    // SAFETY: these entries belong to the locked current root; detached tables remain allocated
    // until after the local and remote TLB invalidations complete.
    unsafe {
        write_volatile(entry, 0);
        if (1..256).contains(&pml4_index) {
            let root = current_address_space() as *mut u64;
            let pml4_entry = root.add(pml4_index);
            let pdpt = (read_volatile(pml4_entry) & PAGE_ADDRESS_MASK) as *mut u64;
            let pdpt_entry = pdpt.add((virtual_address >> 30) & 0x1ff);
            let directory = (read_volatile(pdpt_entry) & PAGE_ADDRESS_MASK) as *mut u64;
            let directory_entry = directory.add((virtual_address >> 21) & 0x1ff);
            let page_table = (entry as usize & !(PAGE_SIZE - 1)) as *mut u64;

            if page_table_is_empty(page_table) {
                write_volatile(directory_entry, 0);
                reclaimed[reclaimed_count] = page_table;
                reclaimed_count += 1;
                if page_table_is_empty(directory) {
                    write_volatile(pdpt_entry, 0);
                    reclaimed[reclaimed_count] = directory;
                    reclaimed_count += 1;
                    if page_table_is_empty(pdpt) {
                        write_volatile(pml4_entry, 0);
                        reclaimed[reclaimed_count] = pdpt;
                        reclaimed_count += 1;
                    }
                }
            }
        }
        core::arch::asm!(
            "invlpg [{}]",
            in(reg) virtual_address,
            options(nostack, preserves_flags),
        );
    }
    shootdown_remote_tlbs();
    drop(_guard);
    for table in reclaimed.into_iter().take(reclaimed_count) {
        if !release_page_table(table) {
            return None;
        }
    }
    Some((current & PAGE_ADDRESS_MASK) as usize)
}

pub(crate) fn page_is_mapped(virtual_address: usize) -> bool {
    if virtual_address % PAGE_SIZE != 0 {
        return false;
    }
    let _guard = PAGE_TABLE_LOCK.lock();
    page_table_value(virtual_address).is_some_and(|entry| entry & PRESENT != 0)
}

pub(crate) fn page_has_backing(virtual_address: usize) -> bool {
    if virtual_address % PAGE_SIZE != 0 {
        return false;
    }
    let _guard = PAGE_TABLE_LOCK.lock();
    page_table_value(virtual_address).is_some_and(|entry| entry & PAGE_ADDRESS_MASK != 0)
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
        let Some(flags) = page_table_value(page) else {
            return false;
        };
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
        let Some(flags) = page_table_value(page) else {
            return false;
        };
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

fn page_table_entry<const N: usize>(
    address: usize,
    create: bool,
    user: bool,
    tables: &mut PageTableReservation<N>,
) -> Option<*mut u64> {
    let upper_bits = address >> 48;
    let sign_bit = (address >> 47) & 1;
    if (sign_bit == 0 && upper_bits != 0) || (sign_bit != 0 && upper_bits != 0xffff) {
        return None;
    }

    let pml4_index = (address >> 39) & 0x1ff;
    let pdpt_index = (address >> 30) & 0x1ff;
    let directory_index = (address >> 21) & 0x1ff;
    let page_index = (address >> 12) & 0x1ff;

    let pml4_table = current_address_space() as *mut u64;
    let pdpt = unsafe { next_table(pml4_table.add(pml4_index), create, 3, user, tables)? };
    let directory = unsafe { next_table(pdpt.add(pdpt_index), create, 2, user, tables)? };
    let page_table = unsafe { next_table(directory.add(directory_index), create, 1, user, tables)? };
    Some(unsafe { page_table.add(page_index) })
}

fn page_table_value(address: usize) -> Option<u64> {
    let upper_bits = address >> 48;
    let sign_bit = (address >> 47) & 1;
    if (sign_bit == 0 && upper_bits != 0) || (sign_bit != 0 && upper_bits != 0xffff) {
        return None;
    }

    let root = current_address_space() as *mut u64;
    // SAFETY: callers hold PAGE_TABLE_LOCK, protecting the active root and its table pages.
    let pml4_entry = unsafe { read_volatile(root.add((address >> 39) & 0x1ff)) };
    if pml4_entry & PRESENT == 0 {
        return None;
    }
    let pdpt = (pml4_entry & PAGE_ADDRESS_MASK) as *mut u64;
    let pdpt_entry = unsafe { read_volatile(pdpt.add((address >> 30) & 0x1ff)) };
    if pdpt_entry & PRESENT == 0 || pdpt_entry & HUGE_PAGE != 0 {
        return (pdpt_entry & (PRESENT | HUGE_PAGE) == (PRESENT | HUGE_PAGE))
            .then_some(pdpt_entry);
    }
    let directory = (pdpt_entry & PAGE_ADDRESS_MASK) as *mut u64;
    let directory_entry = unsafe { read_volatile(directory.add((address >> 21) & 0x1ff)) };
    if directory_entry & PRESENT == 0 || directory_entry & HUGE_PAGE != 0 {
        return (directory_entry & (PRESENT | HUGE_PAGE) == (PRESENT | HUGE_PAGE))
            .then_some(directory_entry);
    }
    let page_table = (directory_entry & PAGE_ADDRESS_MASK) as *mut u64;
    Some(unsafe { read_volatile(page_table.add((address >> 12) & 0x1ff)) })
}

unsafe fn allocate_bootstrap_page_table() -> Option<*mut u64> {
    let used_tables = NEXT_PAGE_TABLE
        .load(Ordering::Acquire)
        .min(BOOTSTRAP_PAGE_TABLE_COUNT);
    let reusable = (0..used_tables).find(|index| {
        PAGE_TABLE_FREE[*index].swap(false, Ordering::AcqRel)
    });
    let table_index = reusable.unwrap_or_else(|| NEXT_PAGE_TABLE.fetch_add(1, Ordering::AcqRel));
    if table_index >= BOOTSTRAP_PAGE_TABLE_COUNT {
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

fn free_bootstrap_page_table(table: *mut u64) -> bool {
    let tables_start = unsafe { addr_of_mut!(PAGE_TABLES).cast::<PageTable>() as usize };
    let Some(offset) = (table as usize).checked_sub(tables_start) else {
        return false;
    };
    if offset % PAGE_SIZE != 0 {
        return false;
    }
    let index = offset / PAGE_SIZE;
    if index >= BOOTSTRAP_PAGE_TABLE_COUNT
        || index >= NEXT_PAGE_TABLE.load(Ordering::Acquire)
        || PAGE_TABLE_FREE[index].swap(true, Ordering::AcqRel)
    {
        return false;
    }
    true
}

fn release_page_table(table: *mut u64) -> bool {
    free_bootstrap_page_table(table)
        || crate::kernel::memory::physical::PhysicalAllocator::deallocate_page_table_frame(
            table as usize,
        )
}

fn page_table_is_empty(table: *mut u64) -> bool {
    // SAFETY: called under PAGE_TABLE_LOCK on a table reached from the current root.
    (0..PAGE_TABLE_ENTRIES).all(|index| unsafe { read_volatile(table.add(index)) == 0 })
}

/// # Safety
/// `table` must be a mapped page-table page protected from mutation for the duration of the walk.
unsafe fn page_table_tree_is_releasable(table: *mut u64, level: usize) -> bool {
    if level <= 1 {
        return true;
    }
    for index in 0..PAGE_TABLE_ENTRIES {
        let entry = unsafe { read_volatile(table.add(index)) };
        if entry & PRESENT == 0 {
            continue;
        }
        if entry & HUGE_PAGE != 0 {
            return false;
        }
        let child = (entry & PAGE_ADDRESS_MASK) as *mut u64;
        if !unsafe { page_table_tree_is_releasable(child, level - 1) } {
            return false;
        }
    }
    true
}

/// # Safety
/// The detached subtree must be exclusively owned and every table page must be identity-mapped.
unsafe fn release_page_table_tree(table: *mut u64, level: usize) -> bool {
    if level > 1 {
        for index in 0..PAGE_TABLE_ENTRIES {
            let entry = unsafe { read_volatile(table.add(index)) };
            if entry & PRESENT == 0 {
                continue;
            }
            let child = (entry & PAGE_ADDRESS_MASK) as *mut u64;
            if !unsafe { release_page_table_tree(child, level - 1) } {
                return false;
            }
        }
    }
    release_page_table(table)
}

unsafe fn next_table<const N: usize>(
    entry: *mut u64,
    create: bool,
    level: usize,
    user: bool,
    tables: &mut PageTableReservation<N>,
) -> Option<*mut u64> {
    let value = unsafe { read_volatile(entry) };
    if value & PRESENT == 0 {
        if !create {
            return None;
        }
        let table = tables.allocate()?;
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
        let table = tables.allocate()?;
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
