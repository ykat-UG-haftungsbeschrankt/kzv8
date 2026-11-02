/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

mod device_tree;
mod entry;
pub(crate) mod interrupts;
mod paging;
mod ram_map;
mod threads;
pub(crate) mod timer;
mod virtio;

pub(crate) use threads::FpuContext;

pub(crate) const MAX_CPUS: usize = 8;
pub(crate) const VM_ARENA_SIZE: usize = 1 << 30;
pub(crate) const VM_BASE: usize = 511usize << 30;
pub(crate) const BOOT_MESSAGE: &[u8] = b"kz: aarch64 Rust kernel booted\n";
pub(crate) const BOOT_INFO_ERROR: &str = "unable to read the device-tree memory map";

/// # Safety
/// `boot_info` must point to the mapped FDT blob supplied by the boot firmware.
pub(crate) unsafe fn initialize_boot_info(boot_info: usize, _boot_magic: usize) -> bool {
    unsafe { device_tree::initialize(boot_info) }
}

pub(crate) const fn virtio_window() -> crate::arch::VirtioWindow {
    virtio::virtio_window()
}

pub(crate) fn initialize_mmu() {
    let memory_regions = crate::memory_resources::memory_regions();
    unsafe {
        paging::initialize();
        ram_map::map_memory_regions(memory_regions.as_slice());
    }
}

pub(crate) fn initialize_thread_stack(stack: &mut [u8], entry: extern "C" fn() -> !) -> usize {
    threads::initialize_stack(stack, entry)
}

pub(crate) fn initialize_thread_local_storage(_cpu_id: usize) -> bool {
    true
}

pub(crate) fn fallback_time_ns() -> u64 {
    0
}

pub(crate) fn dma_sync_for_device() {
    unsafe { core::arch::asm!("dsb sy", options(nostack, preserves_flags)) };
}

pub(crate) fn dma_sync_for_cpu() {
    unsafe { core::arch::asm!("dsb sy", options(nostack, preserves_flags)) };
}

pub(crate) fn decommit_debug(_code: u8) {}

pub(crate) fn is_identity_mapped(address: usize) -> bool {
    let memory_regions = crate::memory_resources::memory_regions();
    address < 0x4000_0000
        || memory_regions.iter().any(|region| {
            region
                .start
                .checked_add(region.length)
                .is_some_and(|end| (region.start..end).contains(&address))
        })
}

pub(crate) fn protect_kernel_image_pages(
    _address: usize,
    _length: usize,
    _permissions: i32,
) -> bool {
    false
}

pub(crate) fn current_thread_stack_bounds(stack_size: usize) -> (*mut core::ffi::c_void, usize) {
    (core::ptr::null_mut(), stack_size)
}

pub(crate) fn high_memory_smoke() -> bool {
    false
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
    ram_map::set_page_permissions(address, length, permissions)
}

pub(crate) fn halt() -> ! {
    loop {
        unsafe {
            core::arch::asm!("wfi", options(nomem, nostack));
        }
    }
}

pub(crate) fn map_page(virtual_address: usize, physical_address: usize, permissions: i32) -> bool {
    ram_map::map_page(virtual_address, physical_address, permissions)
}

pub(crate) fn unmap_page(virtual_address: usize) -> Option<usize> {
    ram_map::unmap_page(virtual_address)
}

pub(crate) fn page_is_mapped(virtual_address: usize) -> bool {
    ram_map::page_is_mapped(virtual_address)
}

pub(crate) fn page_has_backing(virtual_address: usize) -> bool {
    ram_map::page_is_mapped(virtual_address)
}

pub(crate) fn notify_executor() {
    unsafe { core::arch::asm!("sev") };
}

pub(crate) fn wait_for_interrupt() {
    unsafe { core::arch::asm!("msr daifclr, #0xf", "wfe") };
}