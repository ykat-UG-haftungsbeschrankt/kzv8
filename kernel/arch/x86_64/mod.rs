/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) mod debug;
mod acpi;
mod entry;
#[cfg(feature = "intel-vtx")]
pub(crate) mod guest_devices;
pub(crate) mod interrupts;
mod multiboot;
mod paging;
#[allow(dead_code)]
pub(crate) mod pci;
mod ram_map;
mod segments;
mod syscall;
#[cfg(feature = "intel-vtd")]
pub(crate) mod vtd;
#[cfg(feature = "intel-vtx")]
mod vmx;
#[cfg(feature = "amd-v")]
mod svm;
pub(crate) mod smp;
mod threads;
pub(crate) mod timer;
mod virtio;

#[cfg(all(feature = "fpu-x87", feature = "fpu-sse"))]
compile_error!("select exactly one of fpu-x87, fpu-sse, or fpu-avx");

pub(crate) use threads::FpuContext;

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
    timer::ticks()
}

pub(crate) fn dma_sync_for_device() {}

pub(crate) fn dma_sync_for_cpu() {}

pub(crate) fn decommit_debug(code: u8) {
    debug::debugcon(&[code]);
}

pub(crate) fn current_thread_stack_bounds(_stack_size: usize) -> (*mut core::ffi::c_void, usize) {
    let start = core::ptr::addr_of!(stack_bottom) as usize;
    let end = core::ptr::addr_of!(stack_top) as usize;
    (start as *mut core::ffi::c_void, end.saturating_sub(start))
}

pub(crate) fn high_memory_smoke() -> bool {
    const PAGE_SIZE: usize = 4096;
    const HIGH_MEMORY_ALIGNMENT: usize = 4 * 1024 * 1024 * 1024;
    let Some(physical_address) =
        crate::kernel::memory::physical::PhysicalAllocator::allocate_contiguous(
            PAGE_SIZE,
            HIGH_MEMORY_ALIGNMENT,
        )
    else {
        return false;
    };
    let Some(virtual_address) =
        crate::kernel::memory::r#virtual::reserve(PAGE_SIZE, PAGE_SIZE, 2)
    else {
        let _released = crate::kernel::memory::physical::PhysicalAllocator::deallocate_contiguous(
            physical_address,
            PAGE_SIZE,
        );
        return false;
    };
    if !<super::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::map_page(
        virtual_address,
        physical_address,
        2,
    ) {
        let _released = crate::kernel::memory::r#virtual::release(virtual_address, PAGE_SIZE);
        let _released = crate::kernel::memory::physical::PhysicalAllocator::deallocate_contiguous(
            physical_address,
            PAGE_SIZE,
        );
        return false;
    }
    unsafe { core::ptr::write_volatile(virtual_address as *mut u8, 0xa5) };
    let mapped = unsafe { core::ptr::read_volatile(virtual_address as *const u8) == 0xa5 };
    let released = crate::kernel::memory::r#virtual::release(virtual_address, PAGE_SIZE);
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

pub(crate) unsafe fn enter_user(entry: usize, stack_pointer: usize) -> ! {
    unsafe { syscall::enter_user(entry, stack_pointer) }
}

#[allow(dead_code)]
pub(crate) fn initialize_interrupt_controller() -> bool {
    interrupts::initialize_controller()
}

pub(crate) fn halt() -> ! {
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
    }
}

pub(crate) fn notify_executor() {}

pub(crate) fn wait_for_interrupt() {
    unsafe { core::arch::asm!("sti", "hlt") };
}

impl crate::kernel::arch::traits::MemoryArchitecture for super::MemoryArchitecture {
    const VM_ARENA_SIZE: usize = 1 << 36;
    const VM_BASE: usize = 0x10_0000_0000;

    fn physical_address_limit() -> u64 {
        let maximum_leaf = core::arch::x86_64::__cpuid(0x8000_0000).eax;
        let address_bits = if maximum_leaf >= 0x8000_0008 {
            (core::arch::x86_64::__cpuid(0x8000_0008).eax & 0xff).clamp(32, 52)
        } else {
            36
        };
        1u64 << address_bits
    }

    fn identity_mapped_limit() -> u64 {
        ram_map::IDENTITY_MAP_LIMIT as u64
    }

    fn bootstrap_memory_start() -> u64 {
        0
    }

    fn map_boot_memory_region(_region_start: usize, _length: usize) -> bool {
        true
    }

    fn initialize_mmu() {
        paging::initialize();
        unsafe { ram_map::map_high_memory() };
    }

    fn is_identity_mapped(address: usize) -> bool {
        ram_map::is_identity_mapped(address)
    }

    fn protect_kernel_image_pages(address: usize, length: usize, permissions: i32) -> bool {
        const PAGE_SIZE: usize = 4096;
        if permissions != 1 || address % PAGE_SIZE != 0 || length % PAGE_SIZE != 0 {
            return false;
        }
        let Some(end) = address.checked_add(length) else {
            return false;
        };
        let image_start = core::ptr::addr_of!(_start) as usize;
        let image_end = core::ptr::addr_of!(__bss_end) as usize;
        address >= image_start
            && end <= image_end
            && paging::set_page_permissions(address, length, permissions)
    }

    fn set_page_permissions(address: usize, length: usize, permissions: i32) -> bool {
        paging::set_page_permissions(address, length, permissions)
    }

    fn set_user_page_permissions(address: usize, length: usize, permissions: i32) -> bool {
        paging::set_user_page_permissions(address, length, permissions)
    }

    fn map_page(virtual_address: usize, physical_address: usize, permissions: i32) -> bool {
        paging::map_page(virtual_address, physical_address, permissions)
    }

    fn map_user_page(virtual_address: usize, physical_address: usize, permissions: i32) -> bool {
        paging::map_user_page(virtual_address, physical_address, permissions)
    }

    fn unmap_page(virtual_address: usize) -> Option<usize> {
        paging::unmap_page(virtual_address)
    }

    fn page_is_mapped(virtual_address: usize) -> bool {
        paging::page_is_mapped(virtual_address)
    }

    fn page_has_backing(virtual_address: usize) -> bool {
        paging::page_has_backing(virtual_address)
    }

    fn user_range_accessible(address: usize, length: usize, write: bool) -> bool {
        paging::user_range_accessible(address, length, write)
    }

    fn user_range_executable(address: usize, length: usize) -> bool {
        paging::user_range_executable(address, length)
    }

    fn high_memory_smoke() -> bool {
        self::high_memory_smoke()
    }
}

impl crate::kernel::arch::traits::BootArchitecture for super::BootArchitecture {
    const BOOT_MESSAGE: &'static [u8] = b"kz: x86_64 Rust kernel booted\n";
    const BOOT_INFO_ERROR: &'static str = "unable to read the Multiboot memory map";

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

    fn set_user_kernel_stack(stack_pointer: usize) -> bool {
        segments::set_kernel_stack(stack_pointer)
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

    fn wait_for_interrupt() {
        self::wait_for_interrupt();
    }
}

impl crate::kernel::arch::traits::DebugArchitecture for super::DebugArchitecture {
    fn decommit_debug(code: u8) {
        self::decommit_debug(code);
    }
}

#[cfg(feature = "virtualization")]
impl crate::kernel::arch::traits::VirtualizationArchitecture for super::VirtualizationArchitecture {
    #[cfg(feature = "intel-vtx")]
    type GuestContext = vmx::GuestContext;
    #[cfg(not(feature = "intel-vtx"))]
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
        use core::arch::x86_64::__cpuid;

        let maximum_basic_leaf = __cpuid(0).eax;
        let maximum_extended_leaf = __cpuid(0x8000_0000).eax;
        let intel_vtx = if maximum_basic_leaf >= 1 {
            if __cpuid(1).ecx & (1 << 5) != 0 {
                super::VirtualizationCapability::ReportedSupported
            } else {
                super::VirtualizationCapability::ReportedUnsupported
            }
        } else {
            super::VirtualizationCapability::ReportedUnsupported
        };
        let amd_v = if maximum_extended_leaf >= 0x8000_0001 {
            if __cpuid(0x8000_0001).ecx & (1 << 2) != 0 {
                super::VirtualizationCapability::ReportedSupported
            } else {
                super::VirtualizationCapability::ReportedUnsupported
            }
        } else {
            super::VirtualizationCapability::ReportedUnsupported
        };
        #[cfg(feature = "intel-vtd")]
        let intel_vtd = if vtd::probe_units().is_some() {
            super::VirtualizationCapability::ReportedSupported
        } else {
            super::VirtualizationCapability::NotProbed
        };
        #[cfg(not(feature = "intel-vtd"))]
        let intel_vtd = super::VirtualizationCapability::NotProbed;

        super::VirtualizationFeatures {
            host_profile: super::VirtualizationHostProfile::CONFIGURED,
            requested: Self::REQUESTED_FEATURES,
            hardware_report: super::VirtualizationFeatureSet {
                intel_vtx,
                intel_vtd,
                intel_vmdq: super::VirtualizationCapability::NotProbed,
                intel_sriov: super::VirtualizationCapability::NotProbed,
                intel_ddio: super::VirtualizationCapability::NotProbed,
                amd_v,
                aarch64_vhe: super::VirtualizationCapability::NotApplicable,
                aarch64_stage2: super::VirtualizationCapability::NotApplicable,
            },
        }
    }

    fn load_guest_context(
        image: &[u8],
    ) -> Result<Self::GuestContext, super::VirtualizationError> {
        #[cfg(feature = "intel-vtx")]
        {
            vmx::GuestContext::load(image)
        }
        #[cfg(not(feature = "intel-vtx"))]
        {
            let _ = image;
            Err(super::VirtualizationError::Unsupported)
        }
    }

    fn guest_memory_region(context: &Self::GuestContext) -> Option<(usize, usize)> {
        #[cfg(feature = "intel-vtx")]
        {
            Some(context.guest_memory_region())
        }
        #[cfg(not(feature = "intel-vtx"))]
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
        #[cfg(feature = "intel-vtx")]
        {
            context.map_guest_mmio(guest_address, host_address, length)
        }
        #[cfg(not(feature = "intel-vtx"))]
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
        #[cfg(feature = "intel-vtx")]
        {
            context.write_guest_memory(guest_address, bytes)
        }
        #[cfg(not(feature = "intel-vtx"))]
        {
            let _ = (context, guest_address, bytes);
            Err(super::VirtualizationError::Unsupported)
        }
    }

    fn retain_guest_memory(context: &mut Self::GuestContext) {
        #[cfg(feature = "intel-vtx")]
        {
            context.retain_guest_memory();
        }
        #[cfg(not(feature = "intel-vtx"))]
        {
            let _ = context;
        }
    }

    unsafe fn restore_guest_context(
        context: &mut Self::GuestContext,
    ) -> Result<super::VirtualMachineExit, super::VirtualizationError> {
        #[cfg(feature = "intel-vtx")]
        {
            unsafe { context.restore() }
        }
        #[cfg(not(feature = "intel-vtx"))]
        {
            let _ = context;
            Err(super::VirtualizationError::Unsupported)
        }
    }

    fn initialize_host() -> Result<(), super::VirtualizationError> {
        #[cfg(feature = "amd-v")]
        {
            let vendor = core::arch::x86_64::__cpuid(0);
            let amd_vendor = vendor.ebx == u32::from_le_bytes(*b"Auth")
                && vendor.edx == u32::from_le_bytes(*b"enti")
                && vendor.ecx == u32::from_le_bytes(*b"cAMD");
            let hygon_vendor = vendor.ebx == u32::from_le_bytes(*b"Hygo")
                && vendor.edx == u32::from_le_bytes(*b"nGen")
                && vendor.ecx == u32::from_le_bytes(*b"uine");
            if amd_vendor || hygon_vendor {
                return svm::initialize_host();
            }
        }
        Ok(())
    }

    unsafe fn run_guest(
        image: &[u8],
    ) -> Result<super::VirtualMachineExit, super::VirtualizationError> {
        #[cfg(feature = "intel-vtx")]
        {
            unsafe { vmx::run_guest(image) }
        }
        #[cfg(not(feature = "intel-vtx"))]
        {
            let _ = image;
            Err(super::VirtualizationError::Unsupported)
        }
    }
}
