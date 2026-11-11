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

pub(crate) struct BootArchitecture;
pub(crate) struct DeviceArchitecture;
pub(crate) struct ThreadArchitecture;
pub(crate) struct InterruptArchitecture;
pub(crate) struct TimerArchitecture;
pub(crate) struct CpuArchitecture;
pub(crate) struct DebugArchitecture;
pub(crate) struct MemoryArchitecture;

#[cfg(feature = "virtualization")]
#[allow(dead_code)]
pub(crate) struct VirtualizationArchitecture;

#[cfg(feature = "virtualization")]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VirtualizationCapability {
    ReportedSupported,
    ReportedUnsupported,
    NotProbed,
    NotApplicable,
}

#[cfg(feature = "virtualization")]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VirtualizationHostProfile {
    Disabled,
    Minimal,
    Full,
    Both,
}

#[cfg(feature = "virtualization")]
impl VirtualizationHostProfile {
    pub(crate) const CONFIGURED: Self = match (
        cfg!(feature = "virtualization-minimal"),
        cfg!(feature = "virtualization-full"),
    ) {
        (true, true) => Self::Both,
        (true, false) => Self::Minimal,
        (false, true) => Self::Full,
        (false, false) => Self::Disabled,
    };
}

#[cfg(feature = "virtualization")]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VirtualizationFeatureSet<T> {
    pub(crate) intel_vtx: T,
    pub(crate) intel_vtd: T,
    pub(crate) intel_vmdq: T,
    pub(crate) intel_sriov: T,
    pub(crate) intel_ddio: T,
    pub(crate) amd_v: T,
    pub(crate) aarch64_vhe: T,
    pub(crate) aarch64_stage2: T,
}

#[cfg(feature = "virtualization")]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VirtualizationFeatures {
    pub(crate) host_profile: VirtualizationHostProfile,
    pub(crate) requested: VirtualizationFeatureSet<bool>,
    pub(crate) hardware_report: VirtualizationFeatureSet<VirtualizationCapability>,
}

#[cfg(feature = "virtualization")]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VirtualizationError {
    Unsupported,
    InvalidGuestImage,
    ResourceExhausted,
    HostSetupFailed,
    VmxDisabledByFirmware,
    VmxInstructionFailed(u32),
    GuestEntryFailed(u32),
    VmcsSetupFailed,
}

#[cfg(feature = "virtualization")]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VirtualMachineExitReason {
    ExceptionOrNmi,
    ExternalInterrupt,
    Cpuid,
    Rdtsc,
    Halt,
    Vmcall,
    Io,
    EptViolation,
    PreemptionTimer,
    Other(u32),
}

#[cfg(feature = "virtualization")]
#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct GuestRegisters {
    pub(crate) rax: u64,
    pub(crate) rbx: u64,
    pub(crate) rcx: u64,
    pub(crate) rdx: u64,
    pub(crate) rsi: u64,
    pub(crate) rdi: u64,
    pub(crate) rbp: u64,
    pub(crate) r8: u64,
    pub(crate) r9: u64,
    pub(crate) r10: u64,
    pub(crate) r11: u64,
    pub(crate) r12: u64,
    pub(crate) r13: u64,
    pub(crate) r14: u64,
    pub(crate) r15: u64,
}

#[cfg(feature = "virtualization")]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VirtualMachineExit {
    pub(crate) reason: VirtualMachineExitReason,
    pub(crate) qualification: u64,
    pub(crate) instruction_length: u32,
    pub(crate) guest_rip: u64,
    pub(crate) guest_rsp: u64,
    pub(crate) guest_rflags: u64,
    pub(crate) guest_physical_address: Option<u64>,
    pub(crate) interrupt_info: u32,
    pub(crate) registers: GuestRegisters,
}

pub(crate) mod syscall {
    pub(crate) trait SyscallNumbers {
        const EXIT: u64;
        const EXIT_GROUP: u64;
        const GET_PID: u64;
        const GET_TID: u64;
        const SCHED_YIELD: u64;
        const WRITE: u64;
        const CLOCK_GET_TIME: u64;
        const MMAP: u64;
        const MUNMAP: u64;
        const MPROTECT: u64;
    }

    pub(crate) struct Syscall;

    #[allow(non_upper_case_globals)]
    pub(crate) const Exit: u64 = <Syscall as SyscallNumbers>::EXIT;
    #[allow(non_upper_case_globals)]
    pub(crate) const ExitGroup: u64 = <Syscall as SyscallNumbers>::EXIT_GROUP;
    #[allow(non_upper_case_globals)]
    pub(crate) const GetPid: u64 = <Syscall as SyscallNumbers>::GET_PID;
    #[allow(non_upper_case_globals)]
    pub(crate) const GetTid: u64 = <Syscall as SyscallNumbers>::GET_TID;
    #[allow(non_upper_case_globals)]
    pub(crate) const SchedYield: u64 = <Syscall as SyscallNumbers>::SCHED_YIELD;
    #[allow(non_upper_case_globals)]
    pub(crate) const Write: u64 = <Syscall as SyscallNumbers>::WRITE;
    #[allow(non_upper_case_globals)]
    pub(crate) const ClockGetTime: u64 = <Syscall as SyscallNumbers>::CLOCK_GET_TIME;
    #[allow(non_upper_case_globals)]
    pub(crate) const Mmap: u64 = <Syscall as SyscallNumbers>::MMAP;
    #[allow(non_upper_case_globals)]
    pub(crate) const Munmap: u64 = <Syscall as SyscallNumbers>::MUNMAP;
    #[allow(non_upper_case_globals)]
    pub(crate) const Mprotect: u64 = <Syscall as SyscallNumbers>::MPROTECT;
}

pub(crate) mod traits {
    use super::VirtioWindow;

    pub(crate) trait BootArchitecture {
        const BOOT_MESSAGE: &'static [u8];
        const BOOT_INFO_ERROR: &'static str;

        /// # Safety
        /// `boot_info` must point to a valid architecture-specific boot information structure.
        unsafe fn initialize_boot_info(boot_info: usize, boot_magic: usize) -> bool;
    }

    pub(crate) trait DeviceArchitecture {
        const VIRTIO_WINDOW: VirtioWindow;

        fn dma_sync_for_device();
        fn dma_sync_for_cpu();
    }

    pub(crate) trait ThreadArchitecture {
        type FpuContext: Copy;

        fn initialize_thread_stack(stack: &mut [u8], entry: extern "C" fn() -> !) -> usize;
        fn initialize_thread_local_storage(cpu_id: usize) -> bool;
        fn current_thread_stack_bounds(stack_size: usize) -> (*mut core::ffi::c_void, usize);
        /// Enter an already-mapped user image and do not return to the caller.
        ///
        /// # Safety
        /// `entry` and `stack_pointer` must refer to executable and writable user mappings.
        unsafe fn enter_user(entry: usize, stack_pointer: usize) -> !;
        fn switch_thread(
            old_stack: &mut usize,
            new_stack: usize,
            old_fpu_context: &mut Self::FpuContext,
            new_fpu_context: &Self::FpuContext,
        );
    }

    pub(crate) trait InterruptArchitecture {
        /// # Safety
        /// Must be called with interrupts disabled before enabling interrupt delivery.
        unsafe fn initialize_interrupts();
        fn initialize_interrupt_controller() -> bool;
        fn disable_interrupts() -> u64;
        fn enable_interrupts();
        fn restore_interrupts(flags: u64);
        #[allow(dead_code)]
        fn interrupts_enabled() -> bool;
    }

    pub(crate) trait TimerArchitecture {
        fn initialize_timer();
        fn now_ns() -> Option<u64>;
        fn fallback_time_ns() -> u64;
    }

    pub(crate) trait CpuArchitecture {
        fn halt() -> !;
        fn notify_executor();
        fn wait_for_interrupt();
    }

    pub(crate) trait DebugArchitecture {
        fn decommit_debug(code: u8);
    }

    #[cfg(feature = "virtualization")]
    #[allow(dead_code)]
    pub(crate) trait VirtualizationArchitecture {
        type GuestContext;

        const REQUESTED_FEATURES: super::VirtualizationFeatureSet<bool>;

        fn detect_features() -> super::VirtualizationFeatures;

        fn initialize_host() -> Result<(), super::VirtualizationError>;

        fn load_guest_context(
            image: &[u8],
        ) -> Result<Self::GuestContext, super::VirtualizationError>;

        fn guest_memory_region(context: &Self::GuestContext) -> Option<(usize, usize)>;

        fn map_guest_mmio(
            context: &mut Self::GuestContext,
            guest_address: u64,
            host_address: u64,
            length: usize,
        ) -> Result<(), super::VirtualizationError>;

        fn write_guest_memory(
            context: &mut Self::GuestContext,
            guest_address: u64,
            bytes: &[u8],
        ) -> Result<(), super::VirtualizationError>;

        fn retain_guest_memory(context: &mut Self::GuestContext);

        /// # Safety
        /// Guest execution must be allowed by the caller; only trusted guest images may run.
        unsafe fn restore_guest_context(
            context: &mut Self::GuestContext,
        ) -> Result<super::VirtualMachineExit, super::VirtualizationError>;

        /// Run a single-vCPU, 2 MiB raw image for at most 100 ms.
        /// The image starts at guest physical address `0x10000`; registers start zeroed
        /// and the stack page is `0x1ff000` with initial stack pointer `0x200000`.
        /// A polling VirtIO-MMIO console is available at guest physical address `0x10000000`.
        /// A polling VirtIO RTC is available at `0x10001000` and exposes one monotonic clock.
        /// Fixed MMIO devices at `0x10002000`..`0x10008000` are per-VM RAM block, peer-only Ethernet,
        /// RAM SCSI, a read-only `/hello.txt` FUSE tree, serial-backed input, headless 2D GPU, and
        /// balloon PFN tracking. The network has no host uplink, GPU has no display sink, and balloon
        /// does not reclaim host pages.
        /// An assigned PCI BAR is exposed at `0x200000`; its `KZPC` v1 descriptor is at `0x6000`.
        /// PCI discovery, interrupt injection, and firmware are absent; assigned devices poll.
        ///
        /// # Safety
        /// The image executes with guest ring-0 privileges. Only trusted images may be run.
        unsafe fn run_guest(
            image: &[u8],
        ) -> Result<super::VirtualMachineExit, super::VirtualizationError>;
    }

    pub(crate) trait MemoryArchitecture {
        const VM_ARENA_SIZE: usize;
        const VM_BASE: usize;

        fn physical_address_limit() -> u64;
        fn identity_mapped_limit() -> u64;
        fn bootstrap_memory_start() -> u64;
        fn map_boot_memory_region(start: usize, length: usize) -> bool;
        fn initialize_mmu();
        fn is_identity_mapped(address: usize) -> bool;
        fn protect_kernel_image_pages(address: usize, length: usize, permissions: i32) -> bool;
        fn set_page_permissions(address: usize, length: usize, permissions: i32) -> bool;
        fn set_user_page_permissions(address: usize, length: usize, permissions: i32) -> bool;
        fn map_page(virtual_address: usize, physical_address: usize, permissions: i32) -> bool;
        fn map_user_page(virtual_address: usize, physical_address: usize, permissions: i32) -> bool;
        fn unmap_page(virtual_address: usize) -> Option<usize>;
        fn page_is_mapped(virtual_address: usize) -> bool;
        fn page_has_backing(virtual_address: usize) -> bool;
        fn user_range_accessible(address: usize, length: usize, write: bool) -> bool;
        fn user_range_executable(address: usize, length: usize) -> bool;
        fn high_memory_smoke() -> bool;
    }

    pub(crate) trait Syscall {
        const EXIT: u64;
        const EXIT_GROUP: u64;
        const GET_PID: u64;
        const GET_TID: u64;
        const SCHED_YIELD: u64;
        const WRITE: u64;
        const CLOCK_GET_TIME: u64;
        const MMAP: u64;
        const MUNMAP: u64;
        const MPROTECT: u64;
    }
}

#[cfg(target_arch = "aarch64")]
#[path = "aarch64/mod.rs"]
mod arch;
#[cfg(target_arch = "x86_64")]
#[path = "x86_64/mod.rs"]
mod arch;

pub(crate) use arch::*;

#[cfg(target_arch = "x86_64")]
pub(crate) fn start_secondary_cpus() -> usize {
    arch::smp::start_secondary_cpus()
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn start_secondary_cpus() -> usize {
    arch::smp::start_secondary_cpus()
}
