/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VirtioWindow {
    pub(crate) start: usize,
    pub(crate) slots: usize,
    pub(crate) stride: usize,
}

struct BootArchitecture;
pub(crate) trait BootArchitecture {
    const BOOT_MESSAGE: &'static [u8];
    const BOOT_INFO_ERROR: &'static str;

    /// # Safety
    /// `boot_info` must point to a valid architecture-specific boot information structure.
    unsafe fn initialize_boot_info(boot_info: usize, boot_magic: usize) -> bool;
}

struct DeviceArchitecture;
pub(crate) trait DeviceArchitecture {
    const VIRTIO_WINDOW: VirtioWindow;

    fn dma_sync_for_device();
    fn dma_sync_for_cpu();
}

struct ThreadArchitecture;
pub(crate) trait ThreadArchitecture {
    type FpuContext: Copy;

    fn initialize_thread_stack(stack: &mut [u8], entry: extern "C" fn() -> !) -> usize;
    fn initialize_thread_local_storage(cpu_id: usize) -> bool;
    fn current_thread_stack_bounds(stack_size: usize) -> (*mut core::ffi::c_void, usize);
    fn switch_thread(
        old_stack: &mut usize,
        new_stack: usize,
        old_fpu_context: &mut Self::FpuContext,
        new_fpu_context: &Self::FpuContext,
    );
}

struct InterruptArchitecture;
pub(crate) trait InterruptArchitecture {
    /// # Safety
    /// Must be called with interrupts disabled before enabling interrupt delivery.
    unsafe fn initialize_interrupts();
    fn initialize_interrupt_controller() -> bool;
    fn disable_interrupts() -> u64;
    fn enable_interrupts();
    fn restore_interrupts(flags: u64);
    fn interrupts_enabled() -> bool;
}

struct TimerArchitecture;
pub(crate) trait TimerArchitecture {
    fn initialize_timer();
    fn now_ns() -> Option<u64>;
    fn fallback_time_ns() -> u64;
}

struct CpuArchitecture;
pub(crate) trait CpuArchitecture {
    const MAX_CPUS: usize;

    fn halt() -> !;
    fn notify_executor();
    fn wait_for_interrupt();
}

struct DebugArchitecture;
pub(crate) trait DebugArchitecture {
    fn decommit_debug(code: u8);
}

struct MemoryArchitecture;
pub(crate) mod traits {
    pub(crate) trait MemoryArchitecture {
        const VM_ARENA_SIZE: usize;
        const VM_BASE: usize;

        fn initialize_mmu();
        fn is_identity_mapped(address: usize) -> bool;
        fn protect_kernel_image_pages(address: usize, length: usize, permissions: i32) -> bool;
        fn set_page_permissions(address: usize, length: usize, permissions: i32) -> bool;
        fn map_page(virtual_address: usize, physical_address: usize, permissions: i32) -> bool;
        fn unmap_page(virtual_address: usize) -> Option<usize>;
        fn page_is_mapped(virtual_address: usize) -> bool;
        fn page_has_backing(virtual_address: usize) -> bool;
        fn high_memory_smoke() -> bool;
    }
}

#[cfg(target_arch = "aarch64")]
#[path = "aarch64/mod.rs"]
mod arch;
#[cfg(target_arch = "x86_64")]
#[path = "x86_64/mod.rs"]
mod arch;

pub(crate) use arch::*;
