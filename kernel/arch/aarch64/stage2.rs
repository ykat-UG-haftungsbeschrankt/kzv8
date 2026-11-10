/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ptr::{copy_nonoverlapping, write_bytes, write_volatile};

use crate::kernel::arch::traits::{InterruptArchitecture as _, MemoryArchitecture as _};
use crate::kernel::arch::MemoryArchitecture;
use crate::kernel::memory::physical::PhysicalAllocator;

const PAGE_SIZE: usize = 4096;
const GUEST_MEMORY_SIZE: usize = 2 * 1024 * 1024;
const TABLE_PAGES: usize = 3;
const ROOT_OFFSET: usize = 0;
const LEVEL1_OFFSET: usize = PAGE_SIZE;
const LEVEL2_OFFSET: usize = 2 * PAGE_SIZE;
const TABLE_DESCRIPTOR: u64 = 0b11;
const BLOCK_DESCRIPTOR: u64 = 0b01;
const MEMORY_ATTRIBUTE_NORMAL_WB: u64 = 0xf << 2;
const STAGE2_READ_WRITE: u64 = 0b11 << 6;
const INNER_SHAREABLE: u64 = 0b11 << 8;
const ACCESS_FLAG: u64 = 1 << 10;
const HOST_CONTROL_VM: u64 = 1;
const HOST_CONTROL_IMO: u64 = 1 << 4;
const HOST_CONTROL_RW: u64 = 1 << 31;
const STAGE1_MMU_ENABLE: u64 = 1;
const HOST_ENTER_HVC: u64 = 0x40;
const GUEST_EXIT_HVC: u64 = 0x41;
const GUEST_ENTRY: usize = 0x1_0000;
const GUEST_STACK: u64 = GUEST_MEMORY_SIZE as u64;
const EL2_MONITOR_STACK_SIZE: usize = 16 * 1024;

core::arch::global_asm!(
    ".section .text.el2_vectors,\"ax\"",
    ".balign 2048",
    ".global kz_aarch64_el2_vectors",
    "kz_aarch64_el2_vectors:",
    ".rept 8",
    "b kz_aarch64_el2_unhandled",
    ".balign 128",
    ".endr",
    "b kz_aarch64_el2_sync_lower",
    ".balign 128",
    "b kz_aarch64_el2_irq_lower",
    ".balign 128",
    ".rept 6",
    "b kz_aarch64_el2_unhandled",
    ".balign 128",
    ".endr",
    "kz_aarch64_el2_unhandled:",
    "msr daifset, #0xf",
    "1:",
    "wfi",
    "b 1b",
    "kz_aarch64_el2_sync_lower:",
    "sub sp, sp, #816",
    "stp x0, x1, [sp, #0]",
    "stp x2, x3, [sp, #16]",
    "stp x4, x5, [sp, #32]",
    "stp x6, x7, [sp, #48]",
    "stp x8, x9, [sp, #64]",
    "stp x10, x11, [sp, #80]",
    "stp x12, x13, [sp, #96]",
    "stp x14, x15, [sp, #112]",
    "stp x16, x17, [sp, #128]",
    "stp x18, x19, [sp, #144]",
    "stp x20, x21, [sp, #160]",
    "stp x22, x23, [sp, #176]",
    "stp x24, x25, [sp, #192]",
    "stp x26, x27, [sp, #208]",
    "stp x28, x29, [sp, #224]",
    "str x30, [sp, #240]",
    "mrs x0, elr_el2",
    "str x0, [sp, #248]",
    "mrs x0, spsr_el2",
    "str x0, [sp, #256]",
    "mrs x0, sp_el1",
    "str x0, [sp, #264]",
    "mrs x0, sp_el0",
    "str x0, [sp, #272]",
    "str xzr, [sp, #280]",
    "stp q0, q1, [sp, #288]",
    "stp q2, q3, [sp, #320]",
    "stp q4, q5, [sp, #352]",
    "stp q6, q7, [sp, #384]",
    "stp q8, q9, [sp, #416]",
    "stp q10, q11, [sp, #448]",
    "stp q12, q13, [sp, #480]",
    "stp q14, q15, [sp, #512]",
    "stp q16, q17, [sp, #544]",
    "stp q18, q19, [sp, #576]",
    "stp q20, q21, [sp, #608]",
    "stp q22, q23, [sp, #640]",
    "stp q24, q25, [sp, #672]",
    "stp q26, q27, [sp, #704]",
    "stp q28, q29, [sp, #736]",
    "stp q30, q31, [sp, #768]",
    "mrs x0, fpcr",
    "str x0, [sp, #800]",
    "mrs x0, fpsr",
    "str x0, [sp, #808]",
    "mov x0, sp",
    "bl kz_aarch64_el2_dispatch",
    "b kz_aarch64_el2_restore",
    "kz_aarch64_el2_irq_lower:",
    "sub sp, sp, #816",
    "stp x0, x1, [sp, #0]",
    "stp x2, x3, [sp, #16]",
    "stp x4, x5, [sp, #32]",
    "stp x6, x7, [sp, #48]",
    "stp x8, x9, [sp, #64]",
    "stp x10, x11, [sp, #80]",
    "stp x12, x13, [sp, #96]",
    "stp x14, x15, [sp, #112]",
    "stp x16, x17, [sp, #128]",
    "stp x18, x19, [sp, #144]",
    "stp x20, x21, [sp, #160]",
    "stp x22, x23, [sp, #176]",
    "stp x24, x25, [sp, #192]",
    "stp x26, x27, [sp, #208]",
    "stp x28, x29, [sp, #224]",
    "str x30, [sp, #240]",
    "mrs x0, elr_el2",
    "str x0, [sp, #248]",
    "mrs x0, spsr_el2",
    "str x0, [sp, #256]",
    "mrs x0, sp_el1",
    "str x0, [sp, #264]",
    "mrs x0, sp_el0",
    "str x0, [sp, #272]",
    "mov x0, #1",
    "str x0, [sp, #280]",
    "stp q0, q1, [sp, #288]",
    "stp q2, q3, [sp, #320]",
    "stp q4, q5, [sp, #352]",
    "stp q6, q7, [sp, #384]",
    "stp q8, q9, [sp, #416]",
    "stp q10, q11, [sp, #448]",
    "stp q12, q13, [sp, #480]",
    "stp q14, q15, [sp, #512]",
    "stp q16, q17, [sp, #544]",
    "stp q18, q19, [sp, #576]",
    "stp q20, q21, [sp, #608]",
    "stp q22, q23, [sp, #640]",
    "stp q24, q25, [sp, #672]",
    "stp q26, q27, [sp, #704]",
    "stp q28, q29, [sp, #736]",
    "stp q30, q31, [sp, #768]",
    "mrs x0, fpcr",
    "str x0, [sp, #800]",
    "mrs x0, fpsr",
    "str x0, [sp, #808]",
    "mov x0, sp",
    "bl kz_aarch64_el2_dispatch",
    "kz_aarch64_el2_restore:",
    "ldr x0, [sp, #800]",
    "msr fpcr, x0",
    "ldr x0, [sp, #808]",
    "msr fpsr, x0",
    "ldp q30, q31, [sp, #768]",
    "ldp q28, q29, [sp, #736]",
    "ldp q26, q27, [sp, #704]",
    "ldp q24, q25, [sp, #672]",
    "ldp q22, q23, [sp, #640]",
    "ldp q20, q21, [sp, #608]",
    "ldp q18, q19, [sp, #576]",
    "ldp q16, q17, [sp, #544]",
    "ldp q14, q15, [sp, #512]",
    "ldp q12, q13, [sp, #480]",
    "ldp q10, q11, [sp, #448]",
    "ldp q8, q9, [sp, #416]",
    "ldp q6, q7, [sp, #384]",
    "ldp q4, q5, [sp, #352]",
    "ldp q2, q3, [sp, #320]",
    "ldp q0, q1, [sp, #288]",
    "ldp x0, x1, [sp, #0]",
    "ldp x2, x3, [sp, #16]",
    "ldp x4, x5, [sp, #32]",
    "ldp x6, x7, [sp, #48]",
    "ldp x8, x9, [sp, #64]",
    "ldp x10, x11, [sp, #80]",
    "ldp x12, x13, [sp, #96]",
    "ldp x14, x15, [sp, #112]",
    "ldp x16, x17, [sp, #128]",
    "ldp x18, x19, [sp, #144]",
    "ldp x20, x21, [sp, #160]",
    "ldp x22, x23, [sp, #176]",
    "ldp x24, x25, [sp, #192]",
    "ldp x26, x27, [sp, #208]",
    "ldp x28, x29, [sp, #224]",
    "ldr x30, [sp, #240]",
    "ldr x0, [sp, #248]",
    "msr elr_el2, x0",
    "ldr x0, [sp, #256]",
    "msr spsr_el2, x0",
    "ldr x0, [sp, #264]",
    "msr sp_el1, x0",
    "ldr x0, [sp, #272]",
    "msr sp_el0, x0",
    "add sp, sp, #816",
    "eret",
    ".global kz_aarch64_el2_initialize",
    ".type kz_aarch64_el2_initialize,%function",
    "kz_aarch64_el2_initialize:",
    "cmp x0, #8",
    "b.hs 2f",
    "adrp x1, KZ_AARCH64_EL2_MONITOR_STACKS",
    "add x1, x1, :lo12:KZ_AARCH64_EL2_MONITOR_STACKS",
    "add x1, x1, x0, lsl #14",
    "add x1, x1, #4, lsl #12",
    "msr sp_el2, x1",
    "adrp x1, kz_aarch64_el2_vectors",
    "add x1, x1, :lo12:kz_aarch64_el2_vectors",
    "msr vbar_el2, x1",
    "msr tpidr_el2, x0",
    "adrp x1, KZ_AARCH64_EL2_READY",
    "add x1, x1, :lo12:KZ_AARCH64_EL2_READY",
    "mov w2, #0xaa",
    "strb w2, [x1, x0]",
    "isb",
    "2:",
    "ret",
    ".size kz_aarch64_el2_initialize, .-kz_aarch64_el2_initialize",
    ".global kz_aarch64_vcpu_enter",
    ".type kz_aarch64_vcpu_enter,%function",
    "kz_aarch64_vcpu_enter:",
    "hvc #0x40",
    "ret",
    ".size kz_aarch64_vcpu_enter, .-kz_aarch64_vcpu_enter",
    options(raw),
);

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct El2MonitorStack([u8; EL2_MONITOR_STACK_SIZE]);

#[no_mangle]
static mut KZ_AARCH64_EL2_MONITOR_STACKS: [El2MonitorStack; crate::kernel::arch::MAX_CPUS] =
    [const { El2MonitorStack([0; EL2_MONITOR_STACK_SIZE]) }; crate::kernel::arch::MAX_CPUS];

#[no_mangle]
static mut KZ_AARCH64_EL2_READY: [u8; crate::kernel::arch::MAX_CPUS] =
    [0x55; crate::kernel::arch::MAX_CPUS];

#[repr(C)]
pub(crate) struct El2ExceptionFrame {
    pub(crate) x: [u64; 31],
    pub(crate) elr: u64,
    pub(crate) spsr: u64,
    pub(crate) sp_el1: u64,
    pub(crate) sp_el0: u64,
    vector: u64,
    q: [u128; 32],
    fpcr: u64,
    fpsr: u64,
}

#[derive(Clone, Copy)]
struct GuestState {
    x: [u64; 31],
    pc: u64,
    pstate: u64,
    sp: u64,
    q: [u128; 32],
    fpcr: u64,
    fpsr: u64,
}

#[derive(Clone, Copy)]
struct MonitorState {
    active: bool,
    guest_active: bool,
    vtcr: u64,
    vttbr: u64,
    guest: GuestState,
    host_x: [u64; 31],
    host_q: [u128; 32],
    host_fpcr: u64,
    host_fpsr: u64,
    host_pc: u64,
    host_pstate: u64,
    host_sp_el1: u64,
    host_sp_el0: u64,
    host_sctlr_el1: u64,
    host_hcr_el2: u64,
    host_vtcr_el2: u64,
    host_vttbr_el2: u64,
    exit: crate::kernel::arch::VirtualMachineExit,
}

impl MonitorState {
    const EMPTY: Self = Self {
        active: false,
        guest_active: false,
        vtcr: 0,
        vttbr: 0,
        guest: GuestState {
            x: [0; 31],
            pc: 0,
            pstate: 0,
            sp: 0,
            q: [0; 32],
            fpcr: 0,
            fpsr: 0,
        },
        host_x: [0; 31],
        host_q: [0; 32],
        host_fpcr: 0,
        host_fpsr: 0,
        host_pc: 0,
        host_pstate: 0,
        host_sp_el1: 0,
        host_sp_el0: 0,
        host_sctlr_el1: 0,
        host_hcr_el2: 0,
        host_vtcr_el2: 0,
        host_vttbr_el2: 0,
        exit: crate::kernel::arch::VirtualMachineExit {
            reason: crate::kernel::arch::VirtualMachineExitReason::Other(0),
            qualification: 0,
            instruction_length: 0,
            guest_rip: 0,
            guest_rsp: 0,
            guest_rflags: 0,
            guest_physical_address: None,
            interrupt_info: 0,
            registers: crate::kernel::arch::GuestRegisters {
                rax: 0,
                rbx: 0,
                rcx: 0,
                rdx: 0,
                rsi: 0,
                rdi: 0,
                rbp: 0,
                r8: 0,
                r9: 0,
                r10: 0,
                r11: 0,
                r12: 0,
                r13: 0,
                r14: 0,
                r15: 0,
            },
        },
    };
}

static mut KZ_AARCH64_MONITOR_CONTEXTS: [MonitorState; crate::kernel::arch::MAX_CPUS] =
    [const { MonitorState::EMPTY }; crate::kernel::arch::MAX_CPUS];

struct GuestMemory {
    physical: usize,
}

impl GuestMemory {
    fn allocate(image: &[u8]) -> Result<Self, crate::kernel::arch::VirtualizationError> {
        if image.is_empty() || image.len() > GUEST_MEMORY_SIZE - GUEST_ENTRY {
            return Err(crate::kernel::arch::VirtualizationError::InvalidGuestImage);
        }
        let physical = PhysicalAllocator::allocate_contiguous(GUEST_MEMORY_SIZE, GUEST_MEMORY_SIZE)
            .ok_or(crate::kernel::arch::VirtualizationError::ResourceExhausted)?;
        let Some(end) = physical.checked_add(GUEST_MEMORY_SIZE) else {
            let _released = PhysicalAllocator::deallocate_contiguous(physical, GUEST_MEMORY_SIZE);
            return Err(crate::kernel::arch::VirtualizationError::HostSetupFailed);
        };
        if end as u64 > MemoryArchitecture::physical_address_limit()
            || end > MemoryArchitecture::identity_mapped_limit() as usize
            || !MemoryArchitecture::is_identity_mapped(physical)
            || !MemoryArchitecture::is_identity_mapped(end - 1)
        {
            let _released = PhysicalAllocator::deallocate_contiguous(physical, GUEST_MEMORY_SIZE);
            return Err(crate::kernel::arch::VirtualizationError::Unsupported);
        }
        unsafe {
            write_bytes(physical as *mut u8, 0, GUEST_MEMORY_SIZE);
            copy_nonoverlapping(image.as_ptr(), (physical + GUEST_ENTRY) as *mut u8, image.len());
        }
        Ok(Self { physical })
    }
}

impl Drop for GuestMemory {
    fn drop(&mut self) {
        let _released = PhysicalAllocator::deallocate_contiguous(self.physical, GUEST_MEMORY_SIZE);
    }
}

pub(crate) struct GuestContext {
    _memory: GuestMemory,
    tables: Stage2Tables,
    owner_cpu: usize,
    guest: GuestState,
    exit: crate::kernel::arch::VirtualMachineExit,
}

unsafe extern "C" {
    fn kz_aarch64_vcpu_enter(cpu_slot: usize);
}

impl GuestContext {
    pub(crate) fn load(
        image: &[u8],
    ) -> Result<Self, crate::kernel::arch::VirtualizationError> {
        if !monitor_ready() || !hardware_supported() {
            return Err(crate::kernel::arch::VirtualizationError::Unsupported);
        }
        let owner_cpu = current_cpu_slot()
            .ok_or(crate::kernel::arch::VirtualizationError::Unsupported)?;
        let memory = GuestMemory::allocate(image)?;
        let tables = Stage2Tables::identity_map_2m(memory.physical)?;
        Ok(Self {
            _memory: memory,
            tables,
            owner_cpu,
            guest: GuestState {
                x: [0; 31],
                pc: GUEST_ENTRY as u64,
                pstate: 0x5,
                sp: GUEST_STACK,
                q: [0; 32],
                fpcr: 0,
                fpsr: 0,
            },
            exit: empty_guest_exit(),
        })
    }

    /// Resume this vCPU until a VM exit should be returned to its scheduler thread.
    ///
    /// # Safety
    /// Only trusted guest images may be executed.
    pub(crate) unsafe fn restore(
        &mut self,
    ) -> Result<crate::kernel::arch::VirtualMachineExit, crate::kernel::arch::VirtualizationError> {
        if self.owner_cpu != current_cpu_slot()
            .ok_or(crate::kernel::arch::VirtualizationError::Unsupported)?
        {
            return Err(crate::kernel::arch::VirtualizationError::HostSetupFailed);
        }
        let state_pointer = monitor_context_pointer(self.owner_cpu)?;
        let interrupt_state = crate::kernel::arch::InterruptArchitecture::disable_interrupts();
        let mut state = MonitorState::EMPTY;
        state.active = true;
        state.vtcr = self.tables.vtcr();
        state.vttbr = self.tables.vttbr();
        state.guest = self.guest;
        unsafe { core::ptr::write_volatile(state_pointer, state) };
        // SAFETY: this vCPU owns the EL2 monitor slot until the host HVC returns.
        unsafe { kz_aarch64_vcpu_enter(self.owner_cpu) };
        state = unsafe { core::ptr::read_volatile(state_pointer) };
        unsafe {
            core::ptr::write_volatile(
                core::ptr::addr_of_mut!((*state_pointer).active),
                false,
            );
        }
        self.guest = state.guest;
        self.exit = state.exit;
        match self.exit.reason {
            crate::kernel::arch::VirtualMachineExitReason::PreemptionTimer => {
                super::timer::handle_interrupt();
            }
            crate::kernel::arch::VirtualMachineExitReason::ExternalInterrupt => {
                crate::kz_handle_virtio_interrupt();
            }
            _ => {}
        }
        crate::kernel::arch::InterruptArchitecture::restore_interrupts(interrupt_state);
        Ok(self.exit)
    }
}

pub(crate) fn initialize_host() -> Result<(), crate::kernel::arch::VirtualizationError> {
    if monitor_ready() && hardware_supported() {
        Ok(())
    } else {
        Err(crate::kernel::arch::VirtualizationError::Unsupported)
    }
}

fn empty_guest_exit() -> crate::kernel::arch::VirtualMachineExit {
    crate::kernel::arch::VirtualMachineExit {
        reason: crate::kernel::arch::VirtualMachineExitReason::Other(0),
        qualification: 0,
        instruction_length: 0,
        guest_rip: 0,
        guest_rsp: 0,
        guest_rflags: 0,
        guest_physical_address: None,
        interrupt_info: 0,
        registers: crate::kernel::arch::GuestRegisters::default(),
    }
}

fn current_cpu_slot() -> Option<usize> {
    let slot: u64;
    unsafe {
        core::arch::asm!(
            "mrs {slot}, tpidr_el1",
            slot = out(reg) slot,
            options(nomem, nostack, preserves_flags),
        );
    }
    let slot = usize::try_from(slot).ok()?;
    (slot < crate::kernel::arch::MAX_CPUS).then_some(slot)
}

fn current_el2_cpu_slot() -> Option<usize> {
    let slot: u64;
    unsafe {
        core::arch::asm!(
            "mrs {slot}, tpidr_el2",
            slot = out(reg) slot,
            options(nomem, nostack, preserves_flags),
        );
    }
    let slot = usize::try_from(slot).ok()?;
    (slot < crate::kernel::arch::MAX_CPUS).then_some(slot)
}

fn monitor_ready() -> bool {
    current_cpu_slot().is_some_and(|slot| unsafe { KZ_AARCH64_EL2_READY[slot] == 0xaa })
}

fn monitor_context_pointer(slot: usize) -> Result<*mut MonitorState, crate::kernel::arch::VirtualizationError> {
    if slot >= crate::kernel::arch::MAX_CPUS {
        return Err(crate::kernel::arch::VirtualizationError::Unsupported);
    }
    Ok(unsafe { core::ptr::addr_of_mut!(KZ_AARCH64_MONITOR_CONTEXTS).cast::<MonitorState>().add(slot) })
}

#[no_mangle]
extern "C" fn kz_aarch64_el2_dispatch(frame: *mut El2ExceptionFrame) {
    let Some(slot) = current_el2_cpu_slot() else {
        el2_halt();
    };
    let Ok(context_pointer) = monitor_context_pointer(slot) else {
        el2_halt();
    };
    let context = unsafe { &mut *context_pointer };
    if !context.active {
        el2_halt();
    }
    let frame = unsafe { &mut *frame };
    let syndrome = read_esr_el2();
    if context.guest_active {
        let interrupt_id = if frame.vector == 1 {
            super::interrupts::acknowledge_el2_irq()
        } else {
            None
        };
        let reason = if let Some(interrupt_id) = interrupt_id {
            if interrupt_id == super::timer::INTERRUPT_ID {
                crate::kernel::arch::VirtualMachineExitReason::PreemptionTimer
            } else {
                crate::kernel::arch::VirtualMachineExitReason::ExternalInterrupt
            }
        } else if frame.vector == 0 && syndrome >> 26 == 0x16 && syndrome & 0xffff == GUEST_EXIT_HVC {
            crate::kernel::arch::VirtualMachineExitReason::Vmcall
        } else if frame.vector == 0 {
            crate::kernel::arch::VirtualMachineExitReason::Other((syndrome >> 26) as u32)
        } else {
            crate::kernel::arch::VirtualMachineExitReason::ExternalInterrupt
        };
        context.save_guest_exit(frame, reason, syndrome, interrupt_id.unwrap_or(0));
        context.restore_host(frame);
        return;
    }

    if frame.vector != 0
        || syndrome >> 26 != 0x16
        || syndrome & 0xffff != HOST_ENTER_HVC
        || frame.x[0] != slot as u64
    {
        el2_halt();
    }
    context.save_host(frame);
    context.enter_guest(frame);
}

impl MonitorState {
    fn save_host(&mut self, frame: &El2ExceptionFrame) {
        self.host_x = frame.x;
        self.host_q = frame.q;
        self.host_fpcr = frame.fpcr;
        self.host_fpsr = frame.fpsr;
        self.host_pc = frame.elr;
        self.host_pstate = frame.spsr;
        self.host_sp_el1 = frame.sp_el1;
        self.host_sp_el0 = frame.sp_el0;
        self.host_sctlr_el1 = read_sctlr_el1();
        self.host_hcr_el2 = read_hcr_el2();
        self.host_vtcr_el2 = read_vtcr_el2();
        self.host_vttbr_el2 = read_vttbr_el2();
    }

    fn enter_guest(&mut self, frame: &mut El2ExceptionFrame) {
        unsafe {
            core::arch::asm!(
                "msr vtcr_el2, {vtcr}",
                "msr vttbr_el2, {vttbr}",
                "dsb ishst",
                "tlbi vmalls12e1is",
                "dsb ish",
                "isb",
                vtcr = in(reg) self.vtcr,
                vttbr = in(reg) self.vttbr,
                options(nostack, preserves_flags),
            );
            write_sctlr_el1(self.host_sctlr_el1 & !STAGE1_MMU_ENABLE);
            write_hcr_el2(self.host_hcr_el2 | HOST_CONTROL_VM | HOST_CONTROL_IMO | HOST_CONTROL_RW);
        }
        frame.x = self.guest.x;
        frame.elr = self.guest.pc;
        frame.spsr = self.guest.pstate;
        frame.sp_el1 = self.guest.sp;
        frame.sp_el0 = 0;
        frame.q = self.guest.q;
        frame.fpcr = self.guest.fpcr;
        frame.fpsr = self.guest.fpsr;
        self.guest_active = true;
    }

    fn save_guest_exit(
        &mut self,
        frame: &El2ExceptionFrame,
        reason: crate::kernel::arch::VirtualMachineExitReason,
        syndrome: u64,
        interrupt_id: u32,
    ) {
        self.guest.x = frame.x;
        self.guest.pc = frame.elr;
        self.guest.pstate = frame.spsr;
        self.guest.sp = frame.sp_el1;
        self.guest.q = frame.q;
        self.guest.fpcr = frame.fpcr;
        self.guest.fpsr = frame.fpsr;
        let instruction_length = if reason == crate::kernel::arch::VirtualMachineExitReason::Vmcall {
            self.guest.pc = self.guest.pc.wrapping_add(4);
            4
        } else {
            0
        };
        self.exit = crate::kernel::arch::VirtualMachineExit {
            reason,
            qualification: syndrome,
            instruction_length,
            guest_rip: self.guest.pc,
            guest_rsp: self.guest.sp,
            guest_rflags: self.guest.pstate,
            guest_physical_address: None,
            interrupt_info: interrupt_id,
            registers: crate::kernel::arch::GuestRegisters {
                rax: self.guest.x[0],
                rbx: self.guest.x[1],
                rcx: self.guest.x[2],
                rdx: self.guest.x[3],
                rsi: self.guest.x[4],
                rdi: self.guest.x[5],
                rbp: self.guest.x[6],
                r8: self.guest.x[7],
                r9: self.guest.x[8],
                r10: self.guest.x[9],
                r11: self.guest.x[10],
                r12: self.guest.x[11],
                r13: self.guest.x[12],
                r14: self.guest.x[13],
                r15: self.guest.x[14],
            },
        };
    }

    fn restore_host(&mut self, frame: &mut El2ExceptionFrame) {
        unsafe {
            write_hcr_el2(self.host_hcr_el2);
            core::arch::asm!(
                "dsb ishst",
                "tlbi vmalls12e1is",
                "dsb ish",
                "msr vtcr_el2, {vtcr}",
                "msr vttbr_el2, {vttbr}",
                "isb",
                vtcr = in(reg) self.host_vtcr_el2,
                vttbr = in(reg) self.host_vttbr_el2,
                options(nostack, preserves_flags),
            );
            write_sctlr_el1(self.host_sctlr_el1);
        }
        frame.x = self.host_x;
        frame.elr = self.host_pc;
        frame.spsr = self.host_pstate;
        frame.sp_el1 = self.host_sp_el1;
        frame.sp_el0 = self.host_sp_el0;
        frame.q = self.host_q;
        frame.fpcr = self.host_fpcr;
        frame.fpsr = self.host_fpsr;
        self.guest_active = false;
    }
}

fn el2_halt() -> ! {
    loop {
        unsafe { core::arch::asm!("msr daifset, #0xf", "wfi", options(nostack)) };
    }
}

fn read_esr_el2() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {value}, esr_el2",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

fn read_sctlr_el1() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {value}, sctlr_el1",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

unsafe fn write_sctlr_el1(value: u64) {
    unsafe {
        core::arch::asm!(
            "msr sctlr_el1, {value}",
            "isb",
            value = in(reg) value,
            options(nostack, preserves_flags),
        );
    }
}

fn write_hcr_el2(value: u64) {
    unsafe {
        core::arch::asm!(
            "msr hcr_el2, {value}",
            "isb",
            value = in(reg) value,
            options(nostack, preserves_flags),
        );
    }
}

fn read_hcr_el2() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {value}, hcr_el2",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

fn read_vtcr_el2() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {value}, vtcr_el2",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

fn read_vttbr_el2() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {value}, vttbr_el2",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

pub(crate) fn hardware_supported() -> bool {
    let pfr0: u64;
    unsafe {
        core::arch::asm!(
            "mrs {value}, ID_AA64PFR0_EL1",
            value = out(reg) pfr0,
            options(nomem, nostack, preserves_flags),
        );
    }
    ((pfr0 >> 8) & 0xf) != 0xf && ((read_sysreg_id_aa64mmfr0_el1() >> 40) & 0xf) != 0xf
}

pub(crate) struct Stage2Tables {
    tables_physical: usize,
    guest_physical: usize,
    vtcr: u64,
    vttbr: u64,
    active: bool,
    host_hcr: u64,
    host_vtcr: u64,
    host_vttbr: u64,
}

impl Stage2Tables {
    pub(crate) fn identity_map_2m(
        guest_physical: usize,
    ) -> Result<Self, crate::kernel::arch::VirtualizationError> {
        if guest_physical % GUEST_MEMORY_SIZE != 0 {
            return Err(crate::kernel::arch::VirtualizationError::Unsupported);
        }
        let guest_end = guest_physical
            .checked_add(GUEST_MEMORY_SIZE)
            .ok_or(crate::kernel::arch::VirtualizationError::Unsupported)?;
        if guest_end as u64 > MemoryArchitecture::physical_address_limit() {
            return Err(crate::kernel::arch::VirtualizationError::Unsupported);
        }
        let mmfr0 = read_sysreg_id_aa64mmfr0_el1();
        let granule_4k = (mmfr0 >> 40) & 0xf;
        let physical_address_size = (mmfr0 & 0xf).min(5);
        let physical_address_bits = match physical_address_size {
            0 => 32,
            1 => 36,
            2 => 40,
            3 => 42,
            4 => 44,
            _ => 48,
        };
        let physical_limit = 1u64 << physical_address_bits;
        if granule_4k == 0xf || guest_end as u64 > physical_limit {
            return Err(crate::kernel::arch::VirtualizationError::Unsupported);
        }
        let tables_physical = PhysicalAllocator::allocate_contiguous(
            TABLE_PAGES * PAGE_SIZE,
            PAGE_SIZE,
        )
        .ok_or(crate::kernel::arch::VirtualizationError::ResourceExhausted)?;
        let tables_end = tables_physical
            .checked_add(TABLE_PAGES * PAGE_SIZE)
            .ok_or(crate::kernel::arch::VirtualizationError::HostSetupFailed)?;
        if tables_end as u64 > physical_limit
            || !MemoryArchitecture::is_identity_mapped(tables_physical)
            || !MemoryArchitecture::is_identity_mapped(tables_end - 1)
        {
            let _released = PhysicalAllocator::deallocate_contiguous(
                tables_physical,
                TABLE_PAGES * PAGE_SIZE,
            );
            return Err(crate::kernel::arch::VirtualizationError::Unsupported);
        }

        unsafe {
            write_bytes(tables_physical as *mut u8, 0, TABLE_PAGES * PAGE_SIZE);
            write_volatile(
                (tables_physical + ROOT_OFFSET) as *mut u64,
                (tables_physical + LEVEL1_OFFSET) as u64 | TABLE_DESCRIPTOR,
            );
            write_volatile(
                (tables_physical + LEVEL1_OFFSET) as *mut u64,
                (tables_physical + LEVEL2_OFFSET) as u64 | TABLE_DESCRIPTOR,
            );
            write_volatile(
                (tables_physical + LEVEL2_OFFSET) as *mut u64,
                guest_physical as u64
                    | MEMORY_ATTRIBUTE_NORMAL_WB
                    | STAGE2_READ_WRITE
                    | INNER_SHAREABLE
                    | ACCESS_FLAG
                    | BLOCK_DESCRIPTOR,
            );
        }

        let vtcr = 24
            | (2 << 6)
            | (1 << 8)
            | (1 << 10)
            | (0b11 << 12)
            | (physical_address_size << 16)
            | (1 << 23)
            | (1 << 31);
        Ok(Self {
            tables_physical,
            guest_physical,
            vtcr,
            vttbr: tables_physical as u64 | (1 << 48),
            active: false,
            host_hcr: 0,
            host_vtcr: 0,
            host_vttbr: 0,
        })
    }

    pub(crate) const fn vtcr(&self) -> u64 {
        self.vtcr
    }

    pub(crate) const fn vttbr(&self) -> u64 {
        self.vttbr
    }

    pub(crate) const fn guest_physical_base(&self) -> usize {
        self.guest_physical
    }

    /// # Safety
    /// Must run at EL2 before entering a guest using this stage-2 table.
    pub(crate) unsafe fn activate(&mut self) -> Result<(), crate::kernel::arch::VirtualizationError> {
        if current_exception_level() != 2 || self.active {
            return Err(crate::kernel::arch::VirtualizationError::HostSetupFailed);
        }
        self.host_hcr = read_hcr_el2();
        if self.host_hcr & HOST_CONTROL_VM != 0 {
            return Err(crate::kernel::arch::VirtualizationError::Unsupported);
        }
        self.host_vtcr = read_vtcr_el2();
        self.host_vttbr = read_vttbr_el2();
        unsafe {
            core::arch::asm!(
                "msr vtcr_el2, {vtcr}",
                "msr vttbr_el2, {vttbr}",
                "dsb ishst",
                "tlbi vmalls12e1is",
                "dsb ish",
                "isb",
                vtcr = in(reg) self.vtcr,
                vttbr = in(reg) self.vttbr,
                options(nostack, preserves_flags),
            );
            write_hcr_el2(self.host_hcr | HOST_CONTROL_VM);
        }
        self.active = true;
        Ok(())
    }

    /// # Safety
    /// Must run at EL2 after the guest has exited and stopped using stage-2 translation.
    pub(crate) unsafe fn deactivate(&mut self) -> Result<(), crate::kernel::arch::VirtualizationError> {
        if current_exception_level() != 2 || !self.active {
            return Err(crate::kernel::arch::VirtualizationError::HostSetupFailed);
        }
        unsafe {
            write_hcr_el2(self.host_hcr & !HOST_CONTROL_VM);
            core::arch::asm!(
                "msr vtcr_el2, {vtcr}",
                "msr vttbr_el2, {vttbr}",
                "dsb ishst",
                "tlbi vmalls12e1is",
                "dsb ish",
                "isb",
                vtcr = in(reg) self.host_vtcr,
                vttbr = in(reg) self.host_vttbr,
                options(nostack, preserves_flags),
            );
            write_hcr_el2(self.host_hcr);
        }
        self.active = false;
        Ok(())
    }
}

impl Drop for Stage2Tables {
    fn drop(&mut self) {
        if !self.active {
            let _released = PhysicalAllocator::deallocate_contiguous(
                self.tables_physical,
                TABLE_PAGES * PAGE_SIZE,
            );
        }
    }
}

fn current_exception_level() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {value}, CurrentEL",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    (value >> 2) & 0x3
}

fn read_sysreg_id_aa64mmfr0_el1() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {value}, ID_AA64MMFR0_EL1",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

fn read_hcr_el2() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {value}, hcr_el2",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

fn read_vtcr_el2() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {value}, vtcr_el2",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

fn read_vttbr_el2() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {value}, vttbr_el2",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}