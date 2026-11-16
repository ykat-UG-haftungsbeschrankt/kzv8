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
        const GET_PPID: u64;
        const GET_TID: u64;
        const GET_UID: u64;
        const GET_EUID: u64;
        const GET_GID: u64;
        const GET_EGID: u64;
        const GET_CWD: u64;
        const GET_GROUPS: u64;
        const LSEEK: u64;
        const GETRESUID: u64;
        const GETRESGID: u64;
        const SETUID: u64;
        const SETGID: u64;
        const SETREUID: u64;
        const SETREGID: u64;
        const SETRESUID: u64;
        const SETRESGID: u64;
        const SET_GROUPS: u64;
        const UMASK: u64;
        const GETPGID: u64;
        const GETSID: u64;
        const SETPGID: u64;
        const SETSID: u64;
        const KILL: u64;
        const READLINKAT: u64;
        const UNLINKAT: u64;
        const GETRANDOM: u64;
        const NANOSLEEP: u64;
        const UNAME: u64;
        const GETRUSAGE: u64;
        const WAIT4: u64;
        const EXECVE: u64;
        #[cfg(target_arch = "x86_64")]
        const FORK: u64;
        #[cfg(target_arch = "x86_64")]
        const CLONE: u64;
        #[cfg(target_arch = "x86_64")]
        const VFORK: u64;
        const LINKAT: u64;
        const SYMLINKAT: u64;
        const DUP: u64;
        const GETRLIMIT: u64;
        const SETRLIMIT: u64;
        const GETPRIORITY: u64;
        const SETPRIORITY: u64;
        const CHDIR: u64;
        const SYNC: u64;
        const RT_SIGPROCMASK: u64;
        const RT_SIGPENDING: u64;
        const RT_SIGACTION: u64;
        const CHMOD: u64;
        const FCHMOD: u64;
        const CHOWN: u64;
        const FCHOWN: u64;
        const SCHED_YIELD: u64;
        const READ: u64;
        #[cfg(target_arch = "x86_64")]
        const POLL: u64;
        const PREAD64: u64;
        const FUTEX: u64;
        const BRK: u64;
        const WRITE: u64;
        const WRITEV: u64;
        const CLOSE: u64;
        const IOCTL: u64;
        const GETDENTS64: u64;
        const FSTAT: u64;
        const NEWFSTATAT: u64;
        const OPENAT: u64;
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
    pub(crate) const GetPpid: u64 = <Syscall as SyscallNumbers>::GET_PPID;
    #[allow(non_upper_case_globals)]
    pub(crate) const GetTid: u64 = <Syscall as SyscallNumbers>::GET_TID;
    #[allow(non_upper_case_globals)]
    pub(crate) const GetUid: u64 = <Syscall as SyscallNumbers>::GET_UID;
    #[allow(non_upper_case_globals)]
    pub(crate) const GetEuid: u64 = <Syscall as SyscallNumbers>::GET_EUID;
    #[allow(non_upper_case_globals)]
    pub(crate) const GetGid: u64 = <Syscall as SyscallNumbers>::GET_GID;
    #[allow(non_upper_case_globals)]
    pub(crate) const GetEgid: u64 = <Syscall as SyscallNumbers>::GET_EGID;
    #[allow(non_upper_case_globals)]
    pub(crate) const GetCwd: u64 = <Syscall as SyscallNumbers>::GET_CWD;
    #[allow(non_upper_case_globals)]
    pub(crate) const GetGroups: u64 = <Syscall as SyscallNumbers>::GET_GROUPS;
    #[allow(non_upper_case_globals)]
    pub(crate) const Lseek: u64 = <Syscall as SyscallNumbers>::LSEEK;
    #[allow(non_upper_case_globals)]
    pub(crate) const Getresuid: u64 = <Syscall as SyscallNumbers>::GETRESUID;
    #[allow(non_upper_case_globals)]
    pub(crate) const Getresgid: u64 = <Syscall as SyscallNumbers>::GETRESGID;
    #[allow(non_upper_case_globals)]
    pub(crate) const Setuid: u64 = <Syscall as SyscallNumbers>::SETUID;
    #[allow(non_upper_case_globals)]
    pub(crate) const Setgid: u64 = <Syscall as SyscallNumbers>::SETGID;
    #[allow(non_upper_case_globals)]
    pub(crate) const Setreuid: u64 = <Syscall as SyscallNumbers>::SETREUID;
    #[allow(non_upper_case_globals)]
    pub(crate) const Setregid: u64 = <Syscall as SyscallNumbers>::SETREGID;
    #[allow(non_upper_case_globals)]
    pub(crate) const Setresuid: u64 = <Syscall as SyscallNumbers>::SETRESUID;
    #[allow(non_upper_case_globals)]
    pub(crate) const Setresgid: u64 = <Syscall as SyscallNumbers>::SETRESGID;
    #[allow(non_upper_case_globals)]
    pub(crate) const SetGroups: u64 = <Syscall as SyscallNumbers>::SET_GROUPS;
    #[allow(non_upper_case_globals)]
    pub(crate) const Umask: u64 = <Syscall as SyscallNumbers>::UMASK;
    #[allow(non_upper_case_globals)]
    pub(crate) const Getpgid: u64 = <Syscall as SyscallNumbers>::GETPGID;
    #[allow(non_upper_case_globals)]
    pub(crate) const Getsid: u64 = <Syscall as SyscallNumbers>::GETSID;
    #[allow(non_upper_case_globals)]
    pub(crate) const Setpgid: u64 = <Syscall as SyscallNumbers>::SETPGID;
    #[allow(non_upper_case_globals)]
    pub(crate) const Setsid: u64 = <Syscall as SyscallNumbers>::SETSID;
    #[allow(non_upper_case_globals)]
    pub(crate) const Kill: u64 = <Syscall as SyscallNumbers>::KILL;
    #[allow(non_upper_case_globals)]
    pub(crate) const Readlinkat: u64 = <Syscall as SyscallNumbers>::READLINKAT;
    #[allow(non_upper_case_globals)]
    pub(crate) const Unlinkat: u64 = <Syscall as SyscallNumbers>::UNLINKAT;
    #[allow(non_upper_case_globals)]
    pub(crate) const Getrandom: u64 = <Syscall as SyscallNumbers>::GETRANDOM;
    #[allow(non_upper_case_globals)]
    pub(crate) const Nanosleep: u64 = <Syscall as SyscallNumbers>::NANOSLEEP;
    #[allow(non_upper_case_globals)]
    pub(crate) const Uname: u64 = <Syscall as SyscallNumbers>::UNAME;
    #[allow(non_upper_case_globals)]
    pub(crate) const Getrusage: u64 = <Syscall as SyscallNumbers>::GETRUSAGE;
    #[allow(non_upper_case_globals)]
    pub(crate) const Wait4: u64 = <Syscall as SyscallNumbers>::WAIT4;
    #[allow(non_upper_case_globals)]
    pub(crate) const Execve: u64 = <Syscall as SyscallNumbers>::EXECVE;
    #[cfg(target_arch = "x86_64")]
    #[allow(non_upper_case_globals)]
    pub(crate) const Fork: u64 = <Syscall as SyscallNumbers>::FORK;
    #[cfg(target_arch = "x86_64")]
    #[allow(non_upper_case_globals)]
    pub(crate) const Clone: u64 = <Syscall as SyscallNumbers>::CLONE;
    #[cfg(target_arch = "x86_64")]
    #[allow(non_upper_case_globals)]
    pub(crate) const Vfork: u64 = <Syscall as SyscallNumbers>::VFORK;
    #[allow(non_upper_case_globals)]
    pub(crate) const Linkat: u64 = <Syscall as SyscallNumbers>::LINKAT;
    #[allow(non_upper_case_globals)]
    pub(crate) const Symlinkat: u64 = <Syscall as SyscallNumbers>::SYMLINKAT;
    #[allow(non_upper_case_globals)]
    pub(crate) const Dup: u64 = <Syscall as SyscallNumbers>::DUP;
    #[allow(non_upper_case_globals)]
    pub(crate) const Getrlimit: u64 = <Syscall as SyscallNumbers>::GETRLIMIT;
    #[allow(non_upper_case_globals)]
    pub(crate) const Setrlimit: u64 = <Syscall as SyscallNumbers>::SETRLIMIT;
    #[allow(non_upper_case_globals)]
    pub(crate) const Getpriority: u64 = <Syscall as SyscallNumbers>::GETPRIORITY;
    #[allow(non_upper_case_globals)]
    pub(crate) const Setpriority: u64 = <Syscall as SyscallNumbers>::SETPRIORITY;
    #[allow(non_upper_case_globals)]
    pub(crate) const Chdir: u64 = <Syscall as SyscallNumbers>::CHDIR;
    #[allow(non_upper_case_globals)]
    pub(crate) const Sync: u64 = <Syscall as SyscallNumbers>::SYNC;
    #[allow(non_upper_case_globals)]
    pub(crate) const RtSigprocmask: u64 = <Syscall as SyscallNumbers>::RT_SIGPROCMASK;
    #[allow(non_upper_case_globals)]
    pub(crate) const RtSigpending: u64 = <Syscall as SyscallNumbers>::RT_SIGPENDING;
    #[allow(non_upper_case_globals)]
    pub(crate) const RtSigaction: u64 = <Syscall as SyscallNumbers>::RT_SIGACTION;
    #[allow(non_upper_case_globals)]
    pub(crate) const Chmod: u64 = <Syscall as SyscallNumbers>::CHMOD;
    #[allow(non_upper_case_globals)]
    pub(crate) const Fchmod: u64 = <Syscall as SyscallNumbers>::FCHMOD;
    #[allow(non_upper_case_globals)]
    pub(crate) const Chown: u64 = <Syscall as SyscallNumbers>::CHOWN;
    #[allow(non_upper_case_globals)]
    pub(crate) const Fchown: u64 = <Syscall as SyscallNumbers>::FCHOWN;
    #[allow(non_upper_case_globals)]
    pub(crate) const SchedYield: u64 = <Syscall as SyscallNumbers>::SCHED_YIELD;
    #[allow(non_upper_case_globals)]
    pub(crate) const Read: u64 = <Syscall as SyscallNumbers>::READ;
    #[cfg(target_arch = "x86_64")]
    #[allow(non_upper_case_globals)]
    pub(crate) const Poll: u64 = <Syscall as SyscallNumbers>::POLL;
    #[allow(non_upper_case_globals)]
    pub(crate) const Pread64: u64 = <Syscall as SyscallNumbers>::PREAD64;
    #[allow(non_upper_case_globals)]
    pub(crate) const Futex: u64 = <Syscall as SyscallNumbers>::FUTEX;
    #[allow(non_upper_case_globals)]
    pub(crate) const Brk: u64 = <Syscall as SyscallNumbers>::BRK;
    #[allow(non_upper_case_globals)]
    pub(crate) const Write: u64 = <Syscall as SyscallNumbers>::WRITE;
    #[allow(non_upper_case_globals)]
    pub(crate) const Writev: u64 = <Syscall as SyscallNumbers>::WRITEV;
    #[allow(non_upper_case_globals)]
    pub(crate) const Close: u64 = <Syscall as SyscallNumbers>::CLOSE;
    #[allow(non_upper_case_globals)]
    pub(crate) const Ioctl: u64 = <Syscall as SyscallNumbers>::IOCTL;
    #[allow(non_upper_case_globals)]
    pub(crate) const GetDents64: u64 = <Syscall as SyscallNumbers>::GETDENTS64;
    #[allow(non_upper_case_globals)]
    pub(crate) const Fstat: u64 = <Syscall as SyscallNumbers>::FSTAT;
    #[allow(non_upper_case_globals)]
    pub(crate) const Newfstatat: u64 = <Syscall as SyscallNumbers>::NEWFSTATAT;
    #[allow(non_upper_case_globals)]
    pub(crate) const OpenAt: u64 = <Syscall as SyscallNumbers>::OPENAT;
    #[cfg(target_arch = "x86_64")]
    #[allow(non_upper_case_globals)]
    pub(crate) const Open: u64 = 2;
    #[cfg(target_arch = "x86_64")]
    #[allow(non_upper_case_globals)]
    pub(crate) const Stat: u64 = 4;
    #[cfg(target_arch = "x86_64")]
    #[allow(non_upper_case_globals)]
    pub(crate) const Lstat: u64 = 6;
    #[allow(non_upper_case_globals)]
    pub(crate) const ClockGetTime: u64 = <Syscall as SyscallNumbers>::CLOCK_GET_TIME;
    #[allow(non_upper_case_globals)]
    pub(crate) const Mmap: u64 = <Syscall as SyscallNumbers>::MMAP;
    #[allow(non_upper_case_globals)]
    pub(crate) const Munmap: u64 = <Syscall as SyscallNumbers>::MUNMAP;
    #[allow(non_upper_case_globals)]
    pub(crate) const Mprotect: u64 = <Syscall as SyscallNumbers>::MPROTECT;

    #[cfg(target_arch = "x86_64")]
    #[allow(non_upper_case_globals)]
    pub(crate) const ArchPrctl: u64 = 158;
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
        /// Select the kernel stack used by privilege transitions for a userspace thread.
        /// Architectures without a per-CPU user-entry stack may keep the default no-op.
        fn set_user_kernel_stack(_stack_pointer: usize) -> bool {
            true
        }
        /// Enter an already-mapped user image and do not return to the caller.
        ///
        /// # Safety
        /// `entry` and `stack_pointer` must refer to executable and writable user mappings.
        unsafe fn enter_user(entry: usize, stack_pointer: usize) -> !;
        /// Enter after `set_user_kernel_stack` has selected the privilege-transition stack.
        ///
        /// # Safety
        /// `entry` and `stack_pointer` must refer to executable and writable user mappings,
        /// and the configured kernel stack must remain valid for the userspace thread.
        unsafe fn enter_user_with_configured_kernel_stack(
            entry: usize,
            stack_pointer: usize,
        ) -> ! {
            unsafe { Self::enter_user(entry, stack_pointer) }
        }
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
        fn random_u32() -> Option<u32>;
        fn wait_for_interrupt();
    }

    pub(crate) trait DebugArchitecture {
        fn decommit_debug(code: u8);
        fn write_early(bytes: &[u8]);
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

        fn unmap_guest_mmio(
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
            devices: &mut dyn super::GuestMmioHandler,
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
        /// A host unplug request or detected PCI-function removal clears that descriptor and revokes
        /// the BAR mapping at a vCPU exit; guests must poll the descriptor and stop using the BAR
        /// after it is cleared. No ACK is required. On x86_64 with Intel VT-d, assigned functions are
        /// checked at VM exits and their DMA contexts are revoked even if config space is gone.
        /// Guest PCI discovery, interrupt injection, and firmware are absent; assigned devices poll.
        ///
        /// # Safety
        /// The image executes with guest ring-0 privileges. Only trusted images may be run.
        unsafe fn run_guest(
            image: &[u8],
            devices: &mut dyn super::GuestMmioHandler,
        ) -> Result<super::VirtualMachineExit, super::VirtualizationError>;
    }

    pub(crate) trait MemoryArchitecture {
        const VM_ARENA_SIZE: usize;
        const VM_BASE: usize;

        fn create_address_space() -> Option<(usize, usize, usize)>;
        fn activate_address_space(root: usize) -> bool;
        fn current_address_space() -> usize;
        fn destroy_address_space(root: usize, user_base: usize, user_length: usize) -> bool;
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
        fn debug_page_frame(_virtual_address: usize) -> Option<usize> {
            None
        }
        fn debug_page_table_entries(_virtual_address: usize) -> Option<[u64; 4]> {
            None
        }
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
        const READ: u64;
        const WRITE: u64;
        const CLOSE: u64;
        const OPENAT: u64;
        const CLOCK_GET_TIME: u64;
        const MMAP: u64;
        const MUNMAP: u64;
        const MPROTECT: u64;
    }
}

#[cfg(feature = "virtualization")]
mod virtualization;
#[cfg(feature = "virtualization")]
pub(crate) use virtualization::{GuestMemoryView, GuestMmioHandler};

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
