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

unsafe extern "C" {
    static _start: u8;
    static __bss_end: u8;
    static stack_bottom: u8;
    static stack_top: u8;
}

pub(crate) use threads::FpuContext;

pub(crate) const MAX_CPUS: usize = 8;
pub(crate) const VM_ARENA_SIZE: usize =
    <super::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::VM_ARENA_SIZE;
pub(crate) const VM_BASE: usize =
    <super::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::VM_BASE;
pub(crate) const BOOT_MESSAGE: &[u8] =
    <super::BootArchitecture as crate::kernel::arch::traits::BootArchitecture>::BOOT_MESSAGE;
pub(crate) const BOOT_INFO_ERROR: &str =
    <super::BootArchitecture as crate::kernel::arch::traits::BootArchitecture>::BOOT_INFO_ERROR;

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

pub(crate) fn initialize_thread_local_storage(cpu_id: usize) -> bool {
    threads::initialize_thread_local_storage(cpu_id)
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

pub(crate) fn current_thread_stack_bounds(_stack_size: usize) -> (*mut core::ffi::c_void, usize) {
    let start = core::ptr::addr_of!(stack_bottom) as usize;
    let end = core::ptr::addr_of!(stack_top) as usize;
    (start as *mut core::ffi::c_void, end.saturating_sub(start))
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

impl crate::kernel::arch::traits::MemoryArchitecture for super::MemoryArchitecture {
    const VM_ARENA_SIZE: usize = 64 << 30;
    const VM_BASE: usize = 448usize << 30;

    fn physical_address_limit() -> u64 {
        1u64 << 48
    }

    fn identity_mapped_limit() -> u64 {
        ram_map::IDENTITY_MAP_LIMIT as u64
    }

    fn bootstrap_memory_start() -> u64 {
        1 << 30
    }

    fn map_boot_memory_region(start: usize, length: usize) -> bool {
        if length == 0 {
            return true;
        }
        let Some(end) = start.checked_add(length) else {
            return false;
        };
        let mapped_end = end.min(ram_map::IDENTITY_MAP_LIMIT);
        if start >= mapped_end {
            return true;
        }
        let region = crate::kernel::memory::memblock::MemoryRegion {
            start,
            length: mapped_end - start,
        };
        unsafe { ram_map::map_memory_regions(core::slice::from_ref(&region)) };
        true
    }

    fn initialize_mmu() {
        let memory_regions =
            crate::kernel::memory::memblock::MemBlock::memory_regions();
        unsafe {
            paging::initialize();
            ram_map::map_memory_regions(memory_regions.as_slice());
            if let Some(fdt_region) = device_tree::boot_info_region() {
                ram_map::map_memory_regions(core::slice::from_ref(&fdt_region));
            }
        }
    }

    fn is_identity_mapped(address: usize) -> bool {
        if address >= 512 * 1024 * 1024 * 1024usize {
            return false;
        }
        let memory_regions =
            crate::kernel::memory::memblock::MemBlock::memory_regions();
        address < 0x4000_0000
            || memory_regions.iter().any(|region| {
                region
                    .start
                    .checked_add(region.length)
                    .is_some_and(|end| (region.start..end).contains(&address))
            })
    }

    fn protect_kernel_image_pages(address: usize, length: usize, permissions: i32) -> bool {
        const PAGE_SIZE: usize = crate::kernel::memory::r#virtual::PAGE_SIZE;
        if permissions != 1 || address % PAGE_SIZE != 0 || length % PAGE_SIZE != 0 {
            return false;
        }
        let Some(end) = address.checked_add(length) else {
            return false;
        };
        let (image_start, image_end) = unsafe {
            (
                core::ptr::addr_of!(_start) as usize,
                core::ptr::addr_of!(__bss_end) as usize,
            )
        };
        address >= image_start
            && end <= image_end
            && ram_map::set_page_permissions(address, length, permissions)
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
        ram_map::page_has_backing(virtual_address)
    }

    fn high_memory_smoke() -> bool {
        self::high_memory_smoke()
    }
}

impl crate::kernel::arch::traits::BootArchitecture for super::BootArchitecture {
    const BOOT_MESSAGE: &'static [u8] = b"kz: aarch64 Rust kernel booted\n";
    const BOOT_INFO_ERROR: &'static str = "unable to read the device-tree memory map";

    unsafe fn initialize_boot_info(boot_info: usize, boot_magic: usize) -> bool {
        unsafe { self::initialize_boot_info(boot_info, boot_magic) }
    }
}

impl crate::kernel::arch::traits::DeviceArchitecture for super::DeviceArchitecture {
    const VIRTIO_WINDOW: crate::kernel::arch::VirtioWindow = virtio::virtio_window();

    fn dma_sync_for_device() {
        self::dma_sync_for_device();
    }

    fn dma_sync_for_cpu() {
        self::dma_sync_for_cpu();
    }
}

impl crate::kernel::arch::traits::ThreadArchitecture for super::ThreadArchitecture {
    type FpuContext = threads::FpuContext;

    fn initialize_thread_stack(stack: &mut [u8], entry: extern "C" fn() -> !) -> usize {
        self::initialize_thread_stack(stack, entry)
    }

    fn initialize_thread_local_storage(cpu_id: usize) -> bool {
        self::initialize_thread_local_storage(cpu_id)
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

impl crate::kernel::arch::traits::InterruptArchitecture for super::InterruptArchitecture {
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

impl crate::kernel::arch::traits::TimerArchitecture for super::TimerArchitecture {
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

impl crate::kernel::arch::traits::CpuArchitecture for super::CpuArchitecture {
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

impl crate::kernel::arch::traits::DebugArchitecture for super::DebugArchitecture {
    fn decommit_debug(code: u8) {
        self::decommit_debug(code);
    }
}
