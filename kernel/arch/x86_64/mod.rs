/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) mod debug;
mod entry;
pub(crate) mod interrupts;
mod multiboot;
mod paging;
pub(crate) mod pci;
mod ram_map;
mod threads;
pub(crate) mod timer;
mod virtio;

#[cfg(all(feature = "fpu-x87", feature = "fpu-sse"))]
compile_error!("select exactly one of fpu-x87, fpu-sse, or fpu-avx");
#[cfg(not(any(feature = "fpu-x87", feature = "fpu-sse")))]
compile_error!("select one of fpu-x87, fpu-sse, or fpu-avx");

pub(crate) use threads::FpuContext;

pub(crate) const MAX_CPUS: usize = 8;
pub(crate) const VM_ARENA_SIZE: usize = 1 << 36;
pub(crate) const VM_BASE: usize = 0x10_0000_0000;
pub(crate) const BOOT_MESSAGE: &[u8] = b"kz: x86_64 Rust kernel booted\n";
pub(crate) const BOOT_INFO_ERROR: &str = "unable to read the Multiboot memory map";

unsafe extern "C" {
    static _start: u8;
    static __bss_end: u8;
    static stack_bottom: u8;
    static stack_top: u8;
}

/// # Safety
/// `boot_info` must point to the Multiboot structure supplied by the bootloader.
pub(crate) unsafe fn initialize_boot_info(boot_info: usize, boot_magic: usize) -> bool {
    unsafe { multiboot::initialize(boot_info, boot_magic) }
}

pub(crate) const fn virtio_window() -> crate::arch::VirtioWindow {
    virtio::virtio_window()
}

pub(crate) fn initialize_mmu() {
    paging::initialize();
    unsafe { ram_map::map_high_memory() };
}

pub(crate) fn initialize_thread_stack(stack: &mut [u8], entry: extern "C" fn() -> !) -> usize {
    threads::initialize_stack(stack, entry)
}

pub(crate) fn initialize_thread_local_storage(cpu_id: usize) -> bool {
    threads::initialize_thread_local_storage(cpu_id)
}

pub(crate) fn fallback_time_ns() -> u64 {
    timer::ticks()
}

pub(crate) fn dma_sync_for_device() {}

pub(crate) fn dma_sync_for_cpu() {}

pub(crate) fn decommit_debug(code: u8) {
    debug::debugcon(&[code]);
}

pub(crate) fn is_identity_mapped(address: usize) -> bool {
    address < crate::memory_resources::MAX_PHYSICAL_ADDRESS
}

pub(crate) fn protect_kernel_image_pages(
    address: usize,
    length: usize,
    permissions: i32,
) -> bool {
    const PAGE_SIZE: usize = 4096;
    if permissions != 1 || address % PAGE_SIZE != 0 || length % PAGE_SIZE != 0 {
        return false;
    }
    let Some(end) = address.checked_add(length) else {
        return false;
    };
    let (image_start, image_end) = unsafe {
        (core::ptr::addr_of!(_start) as usize, core::ptr::addr_of!(__bss_end) as usize)
    };
    address >= image_start
        && end <= image_end
        && paging::set_page_permissions(address, length, permissions)
}

pub(crate) fn current_thread_stack_bounds(
    _stack_size: usize,
) -> (*mut core::ffi::c_void, usize) {
    let start = core::ptr::addr_of!(stack_bottom) as usize;
    let end = core::ptr::addr_of!(stack_top) as usize;
    (start as *mut core::ffi::c_void, end.saturating_sub(start))
}

pub(crate) fn high_memory_smoke() -> bool {
    let Some(address) = crate::physical::allocate_contiguous(4096, 4 * 1024 * 1024 * 1024) else {
        return false;
    };
    unsafe { core::ptr::write_volatile(address as *mut u8, 0xa5) };
    let mapped = unsafe { core::ptr::read_volatile(address as *const u8) == 0xa5 };
    let released = crate::physical::deallocate_contiguous(address, 4096);
    mapped && released
}

pub(crate) fn switch_thread(
    old_stack: &mut usize,
    new_stack: usize,
    old_fpu_context: &mut FpuContext,
    new_fpu_context: &FpuContext,
) {
    unsafe { threads::switch(old_stack, new_stack, old_fpu_context, new_fpu_context) }
}

pub(crate) fn initialize_interrupt_controller() -> bool {
    interrupts::initialize_controller()
}

pub(crate) fn set_page_permissions(address: usize, length: usize, permissions: i32) -> bool {
    paging::set_page_permissions(address, length, permissions)
}

pub(crate) fn halt() -> ! {
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
    }
}

pub(crate) fn map_page(virtual_address: usize, physical_address: usize, permissions: i32) -> bool {
    paging::map_page(virtual_address, physical_address, permissions)
}

pub(crate) fn unmap_page(virtual_address: usize) -> Option<usize> {
    paging::unmap_page(virtual_address)
}

pub(crate) fn page_is_mapped(virtual_address: usize) -> bool {
    paging::page_is_mapped(virtual_address)
}

pub(crate) fn page_has_backing(virtual_address: usize) -> bool {
    paging::page_has_backing(virtual_address)
}

pub(crate) fn notify_executor() {}

pub(crate) fn wait_for_interrupt() {
    unsafe { core::arch::asm!("sti", "hlt") };
}