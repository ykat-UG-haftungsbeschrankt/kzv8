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

pub(crate) struct MemoryArch;

pub(crate) const MAX_CPUS: usize = <MemoryArch as crate::kernel::arch::CpuArchitecture>::MAX_CPUS;
pub(crate) const VM_ARENA_SIZE: usize =
    <MemoryArch as crate::kernel::arch::MemoryArchitecture>::VM_ARENA_SIZE;
pub(crate) const VM_BASE: usize = <MemoryArch as crate::kernel::arch::MemoryArchitecture>::VM_BASE;
pub(crate) const BOOT_MESSAGE: &[u8] =
    <MemoryArch as crate::kernel::arch::BootArchitecture>::BOOT_MESSAGE;
pub(crate) const BOOT_INFO_ERROR: &str =
    <MemoryArch as crate::kernel::arch::BootArchitecture>::BOOT_INFO_ERROR;

/// # Safety
/// `boot_info` must point to the mapped FDT blob supplied by the boot firmware.
pub(crate) unsafe fn initialize_boot_info(boot_info: usize, _boot_magic: usize) -> bool {
    unsafe { device_tree::initialize(boot_info) }
}

pub(crate) const fn virtio_window() -> crate::kernel::arch::VirtioWindow {
    virtio::virtio_window()
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

pub(crate) fn halt() -> ! {
    loop {
        unsafe {
            core::arch::asm!("wfi", options(nomem, nostack));
        }
    }
}

pub(crate) fn notify_executor() {
    unsafe { core::arch::asm!("sev") };
}

pub(crate) fn wait_for_interrupt() {
    unsafe { core::arch::asm!("msr daifclr, #0xf", "wfe") };
}

impl crate::kernel::arch::MemoryArchitecture for MemoryArch {
    const VM_ARENA_SIZE: usize = VM_ARENA_SIZE;
    const VM_BASE: usize = VM_BASE;

    fn initialize_mmu() {
        let memory_regions =
            crate::kernel::memory::resources::MemoryRegionRegistry::memory_regions();
        unsafe {
            paging::initialize();
            ram_map::map_memory_regions(memory_regions.as_slice());
        }
    }

    fn is_identity_mapped(address: usize) -> bool {
        let memory_regions =
            crate::kernel::memory::resources::MemoryRegionRegistry::memory_regions();
        address < 0x4000_0000
            || memory_regions.iter().any(|region| {
                region
                    .start
                    .checked_add(region.length)
                    .is_some_and(|end| (region.start..end).contains(&address))
            })
    }

    fn protect_kernel_image_pages(_address: usize, _length: usize, _permissions: i32) -> bool {
        false
    }

    fn set_page_permissions(address: usize, length: usize, permissions: i32) -> bool {
        ram_map::set_page_permissions(address, length, permissions)
    }

    fn map_page(virtual_address: usize, physical_address: usize, permissions: i32) -> bool {
        ram_map::map_page(virtual_address, physical_address, permissions)
    }

    fn unmap_page(virtual_address: usize) -> Option<usize> {
        ram_map::unmap_page(virtual_address)
    }

    fn page_is_mapped(virtual_address: usize) -> bool {
        ram_map::page_is_mapped(virtual_address)
    }

    fn page_has_backing(virtual_address: usize) -> bool {
        ram_map::page_is_mapped(virtual_address)
    }

    fn high_memory_smoke() -> bool {
        self::high_memory_smoke()
    }
}

impl crate::kernel::arch::BootArchitecture for MemoryArch {
    const BOOT_MESSAGE: &'static [u8] = BOOT_MESSAGE;
    const BOOT_INFO_ERROR: &'static str = BOOT_INFO_ERROR;

    unsafe fn initialize_boot_info(boot_info: usize, boot_magic: usize) -> bool {
        unsafe { self::initialize_boot_info(boot_info, boot_magic) }
    }
}

impl crate::kernel::arch::DeviceArchitecture for MemoryArch {
    const VIRTIO_WINDOW: crate::kernel::arch::VirtioWindow = virtio::virtio_window();

    fn dma_sync_for_device() {
        self::dma_sync_for_device();
    }

    fn dma_sync_for_cpu() {
        self::dma_sync_for_cpu();
    }
}

impl crate::kernel::arch::ThreadArchitecture for MemoryArch {
    type FpuContext = threads::FpuContext;

    fn initialize_thread_stack(stack: &mut [u8], entry: extern "C" fn() -> !) -> usize {
        self::initialize_thread_stack(stack, entry)
    }

    fn initialize_thread_local_storage(_cpu_id: usize) -> bool {
        self::initialize_thread_local_storage(_cpu_id)
    }

    fn current_thread_stack_bounds(stack_size: usize) -> (*mut core::ffi::c_void, usize) {
        self::current_thread_stack_bounds(stack_size)
    }

    fn switch_thread(
        old_stack: &mut usize,
        new_stack: usize,
        old_fpu_context: &mut Self::FpuContext,
        new_fpu_context: &Self::FpuContext,
    ) {
        self::switch_thread(old_stack, new_stack, old_fpu_context, new_fpu_context);
    }
}

impl crate::kernel::arch::InterruptArchitecture for MemoryArch {
    unsafe fn initialize_interrupts() {
        unsafe { interrupts::initialize() };
    }

    fn initialize_interrupt_controller() -> bool {
        interrupts::initialize_controller()
    }

    fn disable_interrupts() -> u64 {
        interrupts::disable()
    }

    fn enable_interrupts() {
        interrupts::enable();
    }

    fn restore_interrupts(flags: u64) {
        interrupts::restore(flags);
    }

    fn interrupts_enabled() -> bool {
        interrupts::enabled()
    }
}

impl crate::kernel::arch::TimerArchitecture for MemoryArch {
    fn initialize_timer() {
        timer::initialize();
    }

    fn now_ns() -> Option<u64> {
        timer::now_ns()
    }

    fn fallback_time_ns() -> u64 {
        self::fallback_time_ns()
    }
}

impl crate::kernel::arch::CpuArchitecture for MemoryArch {
    const MAX_CPUS: usize = MAX_CPUS;

    fn halt() -> ! {
        self::halt()
    }

    fn notify_executor() {
        self::notify_executor();
    }

    fn wait_for_interrupt() {
        self::wait_for_interrupt();
    }
}

impl crate::kernel::arch::DebugArchitecture for MemoryArch {
    fn decommit_debug(code: u8) {
        self::decommit_debug(code);
    }
}
