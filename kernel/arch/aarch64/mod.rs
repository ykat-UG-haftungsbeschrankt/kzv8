/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

mod device_tree;
mod debug;
mod entry;
pub(crate) mod interrupts;
mod paging;
#[cfg(feature = "virtualization")]
mod stage2;
mod ram_map;
mod syscall;
pub(crate) mod smp;
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
#[allow(dead_code)]
pub(crate) const VM_ARENA_SIZE: usize =
    <super::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::VM_ARENA_SIZE;
#[allow(dead_code)]
pub(crate) const VM_BASE: usize =
    <super::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::VM_BASE;
#[allow(dead_code)]
pub(crate) const BOOT_MESSAGE: &[u8] =
    <super::BootArchitecture as crate::kernel::arch::traits::BootArchitecture>::BOOT_MESSAGE;
#[allow(dead_code)]
pub(crate) const BOOT_INFO_ERROR: &str =
    <super::BootArchitecture as crate::kernel::arch::traits::BootArchitecture>::BOOT_INFO_ERROR;

/// # Safety
/// `boot_info` must point to the mapped FDT blob supplied by the boot firmware.
pub(crate) unsafe fn initialize_boot_info(boot_info: usize, _boot_magic: usize) -> bool {
    unsafe { device_tree::initialize(boot_info) }
}

#[allow(dead_code)]
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

pub(crate) unsafe fn enter_user(entry: usize, stack_pointer: usize) -> ! {
    if entry >= (1usize << 39) || stack_pointer >= (1usize << 39) || stack_pointer % 16 != 0 {
        halt();
    }
    unsafe {
        core::arch::asm!(
            "movi v0.16b, #0",
            "movi v1.16b, #0",
            "movi v2.16b, #0",
            "movi v3.16b, #0",
            "movi v4.16b, #0",
            "movi v5.16b, #0",
            "movi v6.16b, #0",
            "movi v7.16b, #0",
            "movi v8.16b, #0",
            "movi v9.16b, #0",
            "movi v10.16b, #0",
            "movi v11.16b, #0",
            "movi v12.16b, #0",
            "movi v13.16b, #0",
            "movi v14.16b, #0",
            "movi v15.16b, #0",
            "movi v16.16b, #0",
            "movi v17.16b, #0",
            "movi v18.16b, #0",
            "movi v19.16b, #0",
            "movi v20.16b, #0",
            "movi v21.16b, #0",
            "movi v22.16b, #0",
            "movi v23.16b, #0",
            "movi v24.16b, #0",
            "movi v25.16b, #0",
            "movi v26.16b, #0",
            "movi v27.16b, #0",
            "movi v28.16b, #0",
            "movi v29.16b, #0",
            "movi v30.16b, #0",
            "movi v31.16b, #0",
            "msr fpcr, xzr",
            "msr fpsr, xzr",
            "msr sp_el0, {stack_pointer}",
            "msr elr_el1, {entry}",
            "msr spsr_el1, xzr",
            "isb",
            "eret",
            stack_pointer = in(reg) stack_pointer,
            entry = in(reg) entry,
            options(noreturn),
        );
    }
}

#[allow(dead_code)]
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

pub(crate) fn random_u32() -> Option<u32> {
    let feature_register: u64;
    // SAFETY: ID_AA64ISAR0_EL1 is readable at EL1 and gates use of the optional RNDR register.
    unsafe {
        core::arch::asm!(
            "mrs {feature_register}, id_aa64isar0_el1",
            feature_register = out(reg) feature_register,
            options(nomem, nostack, preserves_flags),
        );
    }
    if (feature_register >> 60) & 0xf == 0 {
        return None;
    }
    for _ in 0..10 {
        let value: u64;
        let success: u32;
        // SAFETY: ID_AA64ISAR0_EL1 reports FEAT_RNG; RNDR failure is detected through NZCV.
        unsafe {
            core::arch::asm!(
                "mrs {value}, S3_3_C2_C4_0",
                "cset {success:w}, ne",
                value = out(reg) value,
                success = lateout(reg) success,
                options(nomem, nostack),
            );
        }
        if success != 0 {
            return Some(value as u32);
        }
    }
    None
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
        let (image_start, image_end) = (
            core::ptr::addr_of!(_start) as usize,
            core::ptr::addr_of!(__bss_end) as usize,
        );
        address >= image_start
            && end <= image_end
            && ram_map::set_page_permissions(address, length, permissions)
    }

    fn create_address_space() -> Option<(usize, usize, usize)> {
        ram_map::create_address_space()
    }

    fn activate_address_space(root: usize) -> bool {
        ram_map::activate_address_space(root)
    }

    fn current_address_space() -> usize {
        ram_map::current_address_space()
    }

    fn destroy_address_space(root: usize, user_base: usize, user_length: usize) -> bool {
        ram_map::destroy_address_space(root, user_base, user_length)
    }

    fn set_page_permissions(address: usize, length: usize, permissions: i32) -> bool {
        ram_map::set_page_permissions(address, length, permissions)
    }

    fn set_user_page_permissions(address: usize, length: usize, permissions: i32) -> bool {
        ram_map::set_user_page_permissions(address, length, permissions)
    }

    fn map_page(virtual_address: usize, physical_address: usize, permissions: i32) -> bool {
        ram_map::map_page(virtual_address, physical_address, permissions)
    }

    fn map_user_page(virtual_address: usize, physical_address: usize, permissions: i32) -> bool {
        ram_map::map_user_page(virtual_address, physical_address, permissions)
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

    fn user_range_accessible(address: usize, length: usize, write: bool) -> bool {
        ram_map::user_range_accessible(address, length, write)
    }

    fn user_range_executable(address: usize, length: usize) -> bool {
        ram_map::user_range_executable(address, length)
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

    unsafe fn enter_user(entry: usize, stack_pointer: usize) -> ! {
        unsafe { self::enter_user(entry, stack_pointer) }
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

    fn random_u32() -> Option<u32> {
        self::random_u32()
    }

    fn wait_for_interrupt() {
        self::wait_for_interrupt();
    }
}

impl crate::kernel::arch::traits::DebugArchitecture for super::DebugArchitecture {
    fn decommit_debug(code: u8) {
        self::decommit_debug(code);
    }

    fn write_early(bytes: &[u8]) {
        self::debug::write_early(bytes);
    }
}

#[cfg(feature = "virtualization")]
impl crate::kernel::arch::traits::VirtualizationArchitecture for super::VirtualizationArchitecture {
    #[cfg(feature = "aarch64-stage2")]
    type GuestContext = stage2::GuestContext;
    #[cfg(not(feature = "aarch64-stage2"))]
    type GuestContext = ();

    const REQUESTED_FEATURES: super::VirtualizationFeatureSet<bool> = super::VirtualizationFeatureSet {
        intel_vtx: cfg!(feature = "intel-vtx"),
        intel_vtd: cfg!(feature = "intel-vtd"),
        intel_vmdq: cfg!(feature = "intel-vmdq"),
        intel_sriov: cfg!(feature = "intel-sriov"),
        intel_ddio: cfg!(feature = "intel-ddio"),
        amd_v: cfg!(feature = "amd-v"),
        aarch64_vhe: cfg!(feature = "aarch64-vhe"),
        aarch64_stage2: cfg!(feature = "aarch64-stage2"),
    };

    fn detect_features() -> super::VirtualizationFeatures {
        let mmfr1_el1: u64;
        unsafe {
            core::arch::asm!(
                "mrs {value}, ID_AA64MMFR1_EL1",
                value = out(reg) mmfr1_el1,
                options(nomem, nostack, preserves_flags),
            );
        }
        let aarch64_vhe = match (mmfr1_el1 >> 8) & 0xf {
            0 => super::VirtualizationCapability::ReportedUnsupported,
            1 => super::VirtualizationCapability::ReportedSupported,
            _ => super::VirtualizationCapability::NotProbed,
        };

        super::VirtualizationFeatures {
            host_profile: super::VirtualizationHostProfile::CONFIGURED,
            requested: Self::REQUESTED_FEATURES,
            hardware_report: super::VirtualizationFeatureSet {
                intel_vtx: super::VirtualizationCapability::NotApplicable,
                intel_vtd: super::VirtualizationCapability::NotApplicable,
                intel_vmdq: super::VirtualizationCapability::NotApplicable,
                intel_sriov: super::VirtualizationCapability::NotApplicable,
                intel_ddio: super::VirtualizationCapability::NotApplicable,
                amd_v: super::VirtualizationCapability::NotApplicable,
                aarch64_vhe,
                aarch64_stage2: if stage2::hardware_supported() {
                    super::VirtualizationCapability::ReportedSupported
                } else {
                    super::VirtualizationCapability::ReportedUnsupported
                },
            },
        }
    }

    fn load_guest_context(
        image: &[u8],
    ) -> Result<Self::GuestContext, super::VirtualizationError> {
        #[cfg(feature = "aarch64-stage2")]
        {
            stage2::GuestContext::load(image)
        }
        #[cfg(not(feature = "aarch64-stage2"))]
        {
            let _ = image;
            Err(super::VirtualizationError::Unsupported)
        }
    }

    fn guest_memory_region(context: &Self::GuestContext) -> Option<(usize, usize)> {
        #[cfg(feature = "aarch64-stage2")]
        {
            Some(context.guest_memory_region())
        }
        #[cfg(not(feature = "aarch64-stage2"))]
        {
            let _ = context;
            None
        }
    }

    fn map_guest_mmio(
        context: &mut Self::GuestContext,
        guest_address: u64,
        host_address: u64,
        length: usize,
    ) -> Result<(), super::VirtualizationError> {
        #[cfg(feature = "aarch64-stage2")]
        {
            context.map_guest_mmio(guest_address, host_address, length)
        }
        #[cfg(not(feature = "aarch64-stage2"))]
        {
            let _ = (context, guest_address, host_address, length);
            Err(super::VirtualizationError::Unsupported)
        }
    }

    fn write_guest_memory(
        context: &mut Self::GuestContext,
        guest_address: u64,
        bytes: &[u8],
    ) -> Result<(), super::VirtualizationError> {
        #[cfg(feature = "aarch64-stage2")]
        {
            context.write_guest_memory(guest_address, bytes)
        }
        #[cfg(not(feature = "aarch64-stage2"))]
        {
            let _ = (context, guest_address, bytes);
            Err(super::VirtualizationError::Unsupported)
        }
    }

    fn retain_guest_memory(context: &mut Self::GuestContext) {
        #[cfg(feature = "aarch64-stage2")]
        {
            context.retain_guest_memory();
        }
        #[cfg(not(feature = "aarch64-stage2"))]
        {
            let _ = context;
        }
    }

    unsafe fn restore_guest_context(
        context: &mut Self::GuestContext,
    ) -> Result<super::VirtualMachineExit, super::VirtualizationError> {
        #[cfg(feature = "aarch64-stage2")]
        {
            unsafe { context.restore() }
        }
        #[cfg(not(feature = "aarch64-stage2"))]
        {
            let _ = context;
            Err(super::VirtualizationError::Unsupported)
        }
    }

    fn initialize_host() -> Result<(), super::VirtualizationError> {
        #[cfg(feature = "aarch64-stage2")]
        {
            stage2::initialize_host()
        }
        #[cfg(not(feature = "aarch64-stage2"))]
        {
            Err(super::VirtualizationError::Unsupported)
        }
    }

    unsafe fn run_guest(
        image: &[u8],
    ) -> Result<super::VirtualMachineExit, super::VirtualizationError> {
        #[cfg(feature = "aarch64-stage2")]
        {
            let mut context = stage2::GuestContext::load(image)?;
            unsafe { context.restore() }
        }
        #[cfg(not(feature = "aarch64-stage2"))]
        {
            let _ = image;
            Err(super::VirtualizationError::Unsupported)
        }
    }
}
