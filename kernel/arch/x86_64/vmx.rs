/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::arch::x86_64::{__cpuid, __cpuid_count};
use core::mem::size_of;
use core::ptr::{copy_nonoverlapping, write_bytes, write_volatile};

use crate::kernel::arch::traits::{InterruptArchitecture as _, MemoryArchitecture as _};
use crate::kernel::arch::{
    GuestRegisters, MemoryArchitecture, VirtualMachineExit, VirtualMachineExitReason,
    VirtualizationError,
};
use crate::kernel::vm::device::virtio::GuestVirtioDevices;
use crate::kernel::vm::device::GuestMemory;
use super::FpuContext;
use crate::kernel::memory::physical::PhysicalAllocator;

const PAGE_SIZE: usize = 4096;
const GUEST_MEMORY_SIZE: usize = 2 * 1024 * 1024;
const GUEST_ENTRY: usize = 0x1_0000;
const GUEST_STACK: u64 = GUEST_MEMORY_SIZE as u64;
const GUEST_DEVICE_APERTURE: u64 = GUEST_MEMORY_SIZE as u64;
const GUEST_GDT: usize = 0x4000;
const GUEST_TSS: usize = 0x5000;
const VMX_CONTROL_PAGES: usize = 10;
const EPT_PML4_OFFSET: usize = 0;
const EPT_PDPT_OFFSET: usize = PAGE_SIZE;
const EPT_PD_OFFSET: usize = 2 * PAGE_SIZE;
const MSR_BITMAP_OFFSET: usize = 3 * PAGE_SIZE;
const VMXON_OFFSET: usize = 4 * PAGE_SIZE;
const VMCS_OFFSET: usize = 5 * PAGE_SIZE;
const ENTRY_MSR_LIST_OFFSET: usize = 6 * PAGE_SIZE;
const EXIT_STORE_MSR_LIST_OFFSET: usize = 7 * PAGE_SIZE;
const EXIT_LOAD_MSR_LIST_OFFSET: usize = 8 * PAGE_SIZE;
const EPT_DEVICE_PT_OFFSET: usize = 9 * PAGE_SIZE;

const IA32_FEATURE_CONTROL: u32 = 0x3a;
const IA32_VMX_BASIC: u32 = 0x480;
const IA32_VMX_PINBASED_CTLS: u32 = 0x481;
const IA32_VMX_PROCBASED_CTLS: u32 = 0x482;
const IA32_VMX_EXIT_CTLS: u32 = 0x483;
const IA32_VMX_ENTRY_CTLS: u32 = 0x484;
const IA32_VMX_CR0_FIXED0: u32 = 0x486;
const IA32_VMX_CR0_FIXED1: u32 = 0x487;
const IA32_VMX_CR4_FIXED0: u32 = 0x488;
const IA32_VMX_CR4_FIXED1: u32 = 0x489;
const IA32_VMX_PROCBASED_CTLS2: u32 = 0x48b;
const IA32_VMX_EPT_VPID_CAP: u32 = 0x48c;
const IA32_VMX_MISC: u32 = 0x485;
const IA32_VMX_TRUE_PINBASED_CTLS: u32 = 0x48d;
const IA32_VMX_TRUE_PROCBASED_CTLS: u32 = 0x48e;
const IA32_VMX_TRUE_EXIT_CTLS: u32 = 0x48f;
const IA32_VMX_TRUE_ENTRY_CTLS: u32 = 0x490;
const IA32_EFER: u32 = 0xc000_0080;
const IA32_FS_BASE: u32 = 0xc000_0100;
const IA32_GS_BASE: u32 = 0xc000_0101;
const IA32_KERNEL_GS_BASE: u32 = 0xc000_0102;
const IA32_SYSENTER_CS: u32 = 0x174;
const IA32_SYSENTER_ESP: u32 = 0x175;
const IA32_SYSENTER_EIP: u32 = 0x176;

const VMCS_PIN_BASED_CONTROLS: u64 = 0x4000;
const VMCS_PRIMARY_PROCESSOR_CONTROLS: u64 = 0x4002;
const VMCS_EXCEPTION_BITMAP: u64 = 0x4004;
const VMCS_VM_EXIT_CONTROLS: u64 = 0x400c;
const VMCS_VM_EXIT_MSR_STORE_COUNT: u64 = 0x400e;
const VMCS_VM_EXIT_MSR_LOAD_COUNT: u64 = 0x4010;
const VMCS_VM_ENTRY_CONTROLS: u64 = 0x4012;
const VMCS_VM_ENTRY_MSR_LOAD_COUNT: u64 = 0x4014;
const VMCS_SECONDARY_PROCESSOR_CONTROLS: u64 = 0x401e;
const VMCS_GUEST_VMCS_LINK_POINTER: u64 = 0x2800;
const VMCS_PREEMPTION_TIMER_VALUE: u64 = 0x482e;
const VMCS_EXIT_REASON: u64 = 0x4402;
const VMCS_EXIT_INTERRUPTION_INFO: u64 = 0x4404;
const VMCS_EXIT_INSTRUCTION_LENGTH: u64 = 0x440c;
const VMCS_VM_INSTRUCTION_ERROR: u64 = 0x4400;
const VMCS_GUEST_ES_SELECTOR: u64 = 0x0800;
const VMCS_GUEST_CS_SELECTOR: u64 = 0x0802;
const VMCS_GUEST_SS_SELECTOR: u64 = 0x0804;
const VMCS_GUEST_DS_SELECTOR: u64 = 0x0806;
const VMCS_GUEST_FS_SELECTOR: u64 = 0x0808;
const VMCS_GUEST_GS_SELECTOR: u64 = 0x080a;
const VMCS_GUEST_LDTR_SELECTOR: u64 = 0x080c;
const VMCS_GUEST_TR_SELECTOR: u64 = 0x080e;
const VMCS_HOST_ES_SELECTOR: u64 = 0x0c00;
const VMCS_HOST_CS_SELECTOR: u64 = 0x0c02;
const VMCS_HOST_SS_SELECTOR: u64 = 0x0c04;
const VMCS_HOST_DS_SELECTOR: u64 = 0x0c06;
const VMCS_HOST_FS_SELECTOR: u64 = 0x0c08;
const VMCS_HOST_GS_SELECTOR: u64 = 0x0c0a;
const VMCS_HOST_TR_SELECTOR: u64 = 0x0c0c;
const VMCS_MSR_BITMAP: u64 = 0x2004;
const VMCS_VM_EXIT_MSR_STORE_ADDRESS: u64 = 0x2006;
const VMCS_VM_EXIT_MSR_LOAD_ADDRESS: u64 = 0x2008;
const VMCS_VM_ENTRY_MSR_LOAD_ADDRESS: u64 = 0x200a;
const VMCS_EPT_POINTER: u64 = 0x201a;
const VMCS_GUEST_PHYSICAL_ADDRESS: u64 = 0x2400;
const VMCS_GUEST_IA32_EFER: u64 = 0x2806;
const VMCS_HOST_IA32_EFER: u64 = 0x2c02;
const VMCS_GUEST_ES_LIMIT: u64 = 0x4800;
const VMCS_GUEST_CS_LIMIT: u64 = 0x4802;
const VMCS_GUEST_SS_LIMIT: u64 = 0x4804;
const VMCS_GUEST_DS_LIMIT: u64 = 0x4806;
const VMCS_GUEST_FS_LIMIT: u64 = 0x4808;
const VMCS_GUEST_GS_LIMIT: u64 = 0x480a;
const VMCS_GUEST_LDTR_LIMIT: u64 = 0x480c;
const VMCS_GUEST_TR_LIMIT: u64 = 0x480e;
const VMCS_GUEST_GDTR_LIMIT: u64 = 0x4810;
const VMCS_GUEST_IDTR_LIMIT: u64 = 0x4812;
const VMCS_GUEST_ES_ACCESS_RIGHTS: u64 = 0x4814;
const VMCS_GUEST_CS_ACCESS_RIGHTS: u64 = 0x4816;
const VMCS_GUEST_SS_ACCESS_RIGHTS: u64 = 0x4818;
const VMCS_GUEST_DS_ACCESS_RIGHTS: u64 = 0x481a;
const VMCS_GUEST_FS_ACCESS_RIGHTS: u64 = 0x481c;
const VMCS_GUEST_GS_ACCESS_RIGHTS: u64 = 0x481e;
const VMCS_GUEST_LDTR_ACCESS_RIGHTS: u64 = 0x4820;
const VMCS_GUEST_TR_ACCESS_RIGHTS: u64 = 0x4822;
const VMCS_GUEST_INTERRUPTIBILITY: u64 = 0x4824;
const VMCS_GUEST_ACTIVITY_STATE: u64 = 0x4826;
const VMCS_GUEST_PENDING_DEBUG_EXCEPTIONS: u64 = 0x6822;
const VMCS_GUEST_SYSENTER_CS: u64 = 0x482a;
const VMCS_GUEST_CR0: u64 = 0x6800;
const VMCS_GUEST_CR3: u64 = 0x6802;
const VMCS_GUEST_CR4: u64 = 0x6804;
const VMCS_GUEST_ES_BASE: u64 = 0x6806;
const VMCS_GUEST_CS_BASE: u64 = 0x6808;
const VMCS_GUEST_SS_BASE: u64 = 0x680a;
const VMCS_GUEST_DS_BASE: u64 = 0x680c;
const VMCS_GUEST_FS_BASE: u64 = 0x680e;
const VMCS_GUEST_GS_BASE: u64 = 0x6810;
const VMCS_GUEST_LDTR_BASE: u64 = 0x6812;
const VMCS_GUEST_TR_BASE: u64 = 0x6814;
const VMCS_GUEST_GDTR_BASE: u64 = 0x6816;
const VMCS_GUEST_IDTR_BASE: u64 = 0x6818;
const VMCS_GUEST_DR7: u64 = 0x681a;
const VMCS_GUEST_RSP: u64 = 0x681c;
const VMCS_GUEST_RIP: u64 = 0x681e;
const VMCS_GUEST_RFLAGS: u64 = 0x6820;
const VMCS_GUEST_SYSENTER_ESP: u64 = 0x6824;
const VMCS_GUEST_SYSENTER_EIP: u64 = 0x6826;
const VMCS_HOST_SYSENTER_CS: u64 = 0x4c00;
const VMCS_HOST_CR0: u64 = 0x6c00;
const VMCS_HOST_CR3: u64 = 0x6c02;
const VMCS_HOST_CR4: u64 = 0x6c04;
const VMCS_HOST_FS_BASE: u64 = 0x6c06;
const VMCS_HOST_GS_BASE: u64 = 0x6c08;
const VMCS_HOST_TR_BASE: u64 = 0x6c0a;
const VMCS_HOST_GDTR_BASE: u64 = 0x6c0c;
const VMCS_HOST_IDTR_BASE: u64 = 0x6c0e;
const VMCS_HOST_SYSENTER_ESP: u64 = 0x6c10;
const VMCS_HOST_SYSENTER_EIP: u64 = 0x6c12;
const VMCS_HOST_RSP: u64 = 0x6c14;
const VMCS_HOST_RIP: u64 = 0x6c16;
const CR0_PG: u64 = 1 << 31;
const CR0_WP: u64 = 1 << 16;
const CR0_TS: u64 = 1 << 3;
const CR0_EM: u64 = 1 << 2;
const CR0_NE: u64 = 1 << 5;
const CR0_PE: u64 = 1;
const CR4_VMXE: u64 = 1 << 13;
const CR4_PAE: u64 = 1 << 5;
const CR4_OSFXSR: u64 = 1 << 9;
const CR4_OSXMMEXCPT: u64 = 1 << 10;
const CR4_LA57: u64 = 1 << 12;
const CR4_OSXSAVE: u64 = 1 << 18;
const CR4_PKE: u64 = 1 << 22;
const CR4_CET: u64 = 1 << 23;
const CR4_PKS: u64 = 1 << 24;
const EFER_LME: u64 = 1 << 8;
const EFER_LMA: u64 = 1 << 10;
const EFER_NXE: u64 = 1 << 11;
const GUEST_TIME_LIMIT_NS: u64 = 100_000_000;
const FPU_MODE: usize = if cfg!(feature = "fpu-x87") {
    0
} else if cfg!(feature = "fpu-avx") {
    2
} else {
    1
};

#[repr(C, align(16))]
struct VmxMsrEntry {
    index: u32,
    reserved: u32,
    value: u64,
}

#[repr(C, align(16))]
struct EptInvalidationDescriptor {
    ept_pointer: u64,
    reserved: u64,
}

#[derive(Clone, Copy)]
struct VmxControls {
    pin_based: u32,
    primary: u32,
    secondary: u32,
    exit: u32,
    entry: u32,
    revision: u32,
    preemption_timer: u32,
    tsc_frequency: u64,
    physical_address_limit: u64,
    revision_requires_32bit_address: bool,
}

struct PhysicalRun {
    address: usize,
    length: usize,
    retained: bool,
}

impl PhysicalRun {
    fn allocate(length: usize, alignment: usize) -> Result<Self, VirtualizationError> {
        let address = PhysicalAllocator::allocate_contiguous(length, alignment)
            .ok_or(VirtualizationError::ResourceExhausted)?;
        let end = address.checked_add(length);
        let physical_limit = MemoryArchitecture::physical_address_limit() as usize;
        let identity_limit = MemoryArchitecture::identity_mapped_limit() as usize;
        let mapped = end.is_some_and(|end| {
            address % alignment == 0
                && end <= physical_limit
                && end <= identity_limit
                && MemoryArchitecture::is_identity_mapped(address)
                && MemoryArchitecture::is_identity_mapped(end - 1)
        });
        if !mapped {
            let _released = PhysicalAllocator::deallocate_contiguous(address, length);
            return Err(VirtualizationError::Unsupported);
        }
        Ok(Self {
            address,
            length,
            retained: false,
        })
    }

    fn pointer(&self) -> *mut u8 {
        self.address as *mut u8
    }

    fn pointer_at(&self, offset: usize) -> *mut u8 {
        self.address.saturating_add(offset) as *mut u8
    }

    fn zero(&self) {
        // SAFETY: the allocator returned writable RAM fully covered by the identity map.
        unsafe { write_bytes(self.pointer(), 0, self.length) };
    }

    fn retain(&mut self) {
        self.retained = true;
    }
}

impl Drop for PhysicalRun {
    fn drop(&mut self) {
        if !self.retained {
            let _released = PhysicalAllocator::deallocate_contiguous(self.address, self.length);
        }
    }
}

struct InterruptGuard(u64);

impl InterruptGuard {
    fn disable() -> Self {
        Self(crate::kernel::arch::InterruptArchitecture::disable_interrupts())
    }
}

impl Drop for InterruptGuard {
    fn drop(&mut self) {
        crate::kernel::arch::InterruptArchitecture::restore_interrupts(self.0);
    }
}

struct Cr4Guard(u64);

impl Cr4Guard {
    fn enable_vmx() -> Result<Self, VirtualizationError> {
        let old_cr0 = read_cr0();
        let old_cr4 = read_cr4();
        let cr0_fixed0 = read_msr(IA32_VMX_CR0_FIXED0);
        let cr0_fixed1 = read_msr(IA32_VMX_CR0_FIXED1);
        let cr4_fixed0 = read_msr(IA32_VMX_CR4_FIXED0);
        let cr4_fixed1 = read_msr(IA32_VMX_CR4_FIXED1);
        if !satisfies_fixed_bits(old_cr0, cr0_fixed0, cr0_fixed1)
            || !satisfies_fixed_bits(old_cr4, cr4_fixed0, cr4_fixed1)
        {
            return Err(VirtualizationError::HostSetupFailed);
        }
        let new_cr4 = old_cr4 | CR4_VMXE;
        if !satisfies_fixed_bits(new_cr4, cr4_fixed0, cr4_fixed1) {
            return Err(VirtualizationError::Unsupported);
        }
        // SAFETY: fixed-bit validation above confirms VMXE is legal on this CPU.
        unsafe { write_cr4(new_cr4) };
        Ok(Self(old_cr4))
    }
}

impl Drop for Cr4Guard {
    fn drop(&mut self) {
        // SAFETY: VMX has been disabled by the later-dropped session guard.
        unsafe { write_cr4(self.0) };
    }
}

struct VmxSession {
    active: bool,
    ept_pointer: u64,
}

impl VmxSession {
    const fn new() -> Self {
        Self {
            active: false,
            ept_pointer: 0,
        }
    }

    fn start(
        &mut self,
        vmxon_operand: *const u64,
        vmcs_operand: *const u64,
        ept_pointer: u64,
        clear_vmcs: bool,
    ) -> Result<(), VirtualizationError> {
        enable_vmx_feature_control()?;
        if !vmxon(vmxon_operand) {
            return Err(VirtualizationError::VmxInstructionFailed(0));
        }
        self.active = true;
        self.ept_pointer = ept_pointer;
        if (clear_vmcs && !vmclear(vmcs_operand)) || !vmptrld(vmcs_operand) {
            return Err(vm_instruction_error());
        }
        Ok(())
    }
}

impl Drop for VmxSession {
    fn drop(&mut self) {
        if self.active {
            if !unsafe { invept_single_context(self.ept_pointer) } {
                loop {
                    // SAFETY: EPT translations must be invalidated before releasing backing pages.
                    unsafe { core::arch::asm!("cli", "hlt", options(nomem, nostack)) };
                }
            }
            // SAFETY: this guard is dropped on the same CPU after any VM exit.
            if !unsafe { vmxoff() } {
                loop {
                    // SAFETY: VMXOFF failure makes releasing VMX regions unsafe; stop this CPU.
                    unsafe { core::arch::asm!("cli", "hlt", options(nomem, nostack)) };
                }
            }
            self.active = false;
        }
    }
}

core::arch::global_asm!(
    ".section .text",
    ".global kz_vmx_launch",
    ".type kz_vmx_launch,@function",
    "kz_vmx_launch:",
    "push rbx",
    "push rbp",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "push rdx",
    "push rsi",
    "push rdi",
    ".if {fpu_mode} == 0",
    "fnsave [rdx]",
    "frstor [rsi]",
    ".elseif {fpu_mode} == 1",
    "fxsave64 [rdx]",
    "fxrstor64 [rsi]",
    ".else",
    "mov r8, rdx",
    "mov eax, 7",
    "xor edx, edx",
    "xsave64 [r8]",
    "mov r8, rsi",
    "mov eax, 7",
    "xor edx, edx",
    "xrstor64 [r8]",
    ".endif",
    "xor eax, eax",
    "xor ebx, ebx",
    "xor ecx, ecx",
    "xor edx, edx",
    "xor esi, esi",
    "xor edi, edi",
    "xor ebp, ebp",
    "xor r8d, r8d",
    "xor r9d, r9d",
    "xor r10d, r10d",
    "xor r11d, r11d",
    "xor r12d, r12d",
    "xor r13d, r13d",
    "xor r14d, r14d",
    "xor r15d, r15d",
    "mov rax, rsp",
    "mov ecx, 0x6c14",
    "vmwrite rax, rcx",
    "setbe al",
    "test al, al",
    "jnz 1f",
    "xor eax, eax",
    "xor ecx, ecx",
    "vmlaunch",
    "setbe al",
    "movzx eax, al",
    "test eax, eax",
    "jnz 1f",
    "jmp 2f",
    "1:",
    "mov eax, 1",
    "mov r10d, eax",
    ".if {fpu_mode} == 0",
    "mov r8, [rsp + 16]",
    "frstor [r8]",
    ".elseif {fpu_mode} == 1",
    "mov r8, [rsp + 16]",
    "fxrstor64 [r8]",
    ".else",
    "mov r8, [rsp + 16]",
    "mov eax, 7",
    "xor edx, edx",
    "xrstor64 [r8]",
    ".endif",
    "mov eax, r10d",
    "2:",
    "add rsp, 24",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbp",
    "pop rbx",
    "ret",
    ".size kz_vmx_launch, .-kz_vmx_launch",
    ".global kz_vmx_resume",
    ".type kz_vmx_resume,@function",
    "kz_vmx_resume:",
    "push rbx",
    "push rbp",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "push rdx",
    "push rsi",
    "push rdi",
    ".if {fpu_mode} == 0",
    "fnsave [rdx]",
    "frstor [rsi]",
    ".elseif {fpu_mode} == 1",
    "fxsave64 [rdx]",
    "fxrstor64 [rsi]",
    ".else",
    "mov r8, rdx",
    "mov eax, 7",
    "xor edx, edx",
    "xsave64 [r8]",
    "mov r8, rsi",
    "mov eax, 7",
    "xor edx, edx",
    "xrstor64 [r8]",
    ".endif",
    "mov rax, rsp",
    "mov ecx, 0x6c14",
    "vmwrite rax, rcx",
    "setbe al",
    "test al, al",
    "jnz 3f",
    "mov r10, [rsp]",
    "mov rax, [r10 + 0]",
    "mov rbx, [r10 + 8]",
    "mov rcx, [r10 + 16]",
    "mov rdx, [r10 + 24]",
    "mov rsi, [r10 + 32]",
    "mov rbp, [r10 + 48]",
    "mov r8, [r10 + 56]",
    "mov r9, [r10 + 64]",
    "mov r11, [r10 + 80]",
    "mov r12, [r10 + 88]",
    "mov r13, [r10 + 96]",
    "mov r14, [r10 + 104]",
    "mov r15, [r10 + 112]",
    "mov rdi, [r10 + 40]",
    "mov r10, [r10 + 72]",
    "vmresume",
    "setbe al",
    "movzx eax, al",
    "test eax, eax",
    "jnz 3f",
    "jmp 4f",
    "3:",
    "mov eax, 1",
    "mov r10d, eax",
    ".if {fpu_mode} == 0",
    "mov r8, [rsp + 16]",
    "frstor [r8]",
    ".elseif {fpu_mode} == 1",
    "mov r8, [rsp + 16]",
    "fxrstor64 [r8]",
    ".else",
    "mov r8, [rsp + 16]",
    "mov eax, 7",
    "xor edx, edx",
    "xrstor64 [r8]",
    ".endif",
    "mov eax, r10d",
    "4:",
    "add rsp, 24",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbp",
    "pop rbx",
    "ret",
    ".size kz_vmx_resume, .-kz_vmx_resume",
    ".global kz_vmx_exit",
    ".type kz_vmx_exit,@function",
    "kz_vmx_exit:",
    "cld",
    "push rax",
    "push rbx",
    "push rcx",
    "push rdx",
    "push rsi",
    "push rdi",
    "push rbp",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "lea r11, [rsp + 120]",
    "mov r8, [r11 + 8]",
    "mov r9, [r11 + 16]",
    ".if {fpu_mode} == 0",
    "fnsave [r8]",
    "frstor [r9]",
    ".elseif {fpu_mode} == 1",
    "fxsave64 [r8]",
    "fxrstor64 [r9]",
    ".else",
    "mov eax, 7",
    "xor edx, edx",
    "xsave64 [r8]",
    "mov eax, 7",
    "xor edx, edx",
    "xrstor64 [r9]",
    ".endif",
    "mov r10, [r11]",
    "mov rax, [rsp + 112]",
    "mov [r10 + 0], rax",
    "mov rax, [rsp + 104]",
    "mov [r10 + 8], rax",
    "mov rax, [rsp + 96]",
    "mov [r10 + 16], rax",
    "mov rax, [rsp + 88]",
    "mov [r10 + 24], rax",
    "mov rax, [rsp + 80]",
    "mov [r10 + 32], rax",
        fpu_mode = const FPU_MODE,
    "mov [r10 + 40], rax",
    "mov rax, [rsp + 64]",
    "mov [r10 + 48], rax",
    "mov rax, [rsp + 56]",
    "mov [r10 + 56], rax",
    "mov rax, [rsp + 48]",
    "mov [r10 + 64], rax",
    "mov rax, [rsp + 40]",
    "mov [r10 + 72], rax",
    "mov rax, [rsp + 32]",
    "mov [r10 + 80], rax",
    "mov rax, [rsp + 24]",
    "mov [r10 + 88], rax",
    "mov rax, [rsp + 16]",
    "mov [r10 + 96], rax",
    "mov rax, [rsp + 8]",
    "mov [r10 + 104], rax",
    "mov rax, [rsp + 0]",
    "mov [r10 + 112], rax",
    "add rsp, 120",
    "xor eax, eax",
    "add rsp, 24",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbp",
    "pop rbx",
    "ret",
    ".size kz_vmx_exit, .-kz_vmx_exit",
    fpu_mode = const FPU_MODE,
);

unsafe extern "C" {
    fn kz_vmx_launch(
        registers: *mut GuestRegisters,
        guest_fpu: *mut FpuContext,
        host_fpu: *mut FpuContext,
    ) -> u64;
    fn kz_vmx_resume(
        registers: *mut GuestRegisters,
        guest_fpu: *mut FpuContext,
        host_fpu: *mut FpuContext,
    ) -> u64;
    fn kz_vmx_exit();
}

pub(crate) struct GuestContext {
    controls: VmxControls,
    guest_memory: PhysicalRun,
    control_memory: PhysicalRun,
    owner_apic_id: u32,
    registers: GuestRegisters,
    fpu_context: FpuContext,
    devices: GuestVirtioDevices,
    launched: bool,
}

impl GuestContext {
    pub(crate) fn load(image: &[u8]) -> Result<Self, VirtualizationError> {
        if image.is_empty() || image.len() > GUEST_MEMORY_SIZE - PAGE_SIZE - GUEST_ENTRY {
            return Err(VirtualizationError::InvalidGuestImage);
        }
        let controls = discover_controls()?;
        let guest_memory = PhysicalRun::allocate(GUEST_MEMORY_SIZE, GUEST_MEMORY_SIZE)?;
        let control_memory = PhysicalRun::allocate(VMX_CONTROL_PAGES * PAGE_SIZE, PAGE_SIZE)?;
        validate_physical_addresses(&controls, &guest_memory, &control_memory)?;
        initialize_guest_memory(&guest_memory, image);
        initialize_control_memory(&control_memory, &guest_memory, &controls);

        let _interrupt_guard = InterruptGuard::disable();
        let host_segments =
            super::segments::initialize().ok_or(VirtualizationError::HostSetupFailed)?;
        let _cr4_guard = Cr4Guard::enable_vmx()?;
        let session = Self::begin_vmx(&control_memory, true)?;
        configure_vmcs(&controls, &control_memory, host_segments)?;
        drop(session);

        Ok(Self {
            controls,
            guest_memory,
            control_memory,
            owner_apic_id: super::interrupts::current_apic_id(),
            registers: GuestRegisters::default(),
            fpu_context: FpuContext::INITIAL,
            devices: GuestVirtioDevices::new()?,
            launched: false,
        })
    }

    pub(crate) fn guest_memory_region(&self) -> (usize, usize) {
        (self.guest_memory.address, self.guest_memory.length)
    }

    pub(crate) fn retain_guest_memory(&mut self) {
        self.guest_memory.retain();
    }

    pub(crate) fn write_guest_memory(
        &mut self,
        guest_address: u64,
        bytes: &[u8],
    ) -> Result<(), VirtualizationError> {
        let start = usize::try_from(guest_address).map_err(|_| VirtualizationError::Unsupported)?;
        let end = start
            .checked_add(bytes.len())
            .ok_or(VirtualizationError::Unsupported)?;
        if end > self.guest_memory.length {
            return Err(VirtualizationError::Unsupported);
        }
        // SAFETY: the checked range is in this context's exclusive guest-memory allocation.
        unsafe {
            copy_nonoverlapping(bytes.as_ptr(), self.guest_memory.pointer_at(start), bytes.len());
        }
        Ok(())
    }

    pub(crate) fn map_guest_mmio(
        &mut self,
        guest_address: u64,
        host_address: u64,
        length: usize,
    ) -> Result<(), VirtualizationError> {
        if guest_address != GUEST_DEVICE_APERTURE
            || length == 0
            || length > GUEST_MEMORY_SIZE
            || length % PAGE_SIZE != 0
            || host_address as usize % PAGE_SIZE != 0
            || host_address
                .checked_add(length as u64)
                .is_none_or(|end| end > self.controls.physical_address_limit)
        {
            return Err(VirtualizationError::Unsupported);
        }
        let page_table = self.control_memory.pointer_at(EPT_DEVICE_PT_OFFSET).cast::<u64>();
        // SAFETY: the requested length is page-bounded within the owned EPT leaf table.
        for page in 0..length / PAGE_SIZE {
            let entry = unsafe { page_table.add(page) };
            if unsafe { core::ptr::read_volatile(entry) } != 0 {
                return Err(VirtualizationError::Unsupported);
            }
        }
        for page in 0..length / PAGE_SIZE {
            let entry = unsafe { page_table.add(page) };
            let physical = host_address + (page * PAGE_SIZE) as u64;
            // SAFETY: the vCPU is stopped and the EPT leaf lies in its owned page-table run.
            unsafe { write_volatile(entry, physical | 0x3) };
        }
        Ok(())
    }

    /// Resume this vCPU until a VM exit should be returned to its scheduler thread.
    ///
    /// # Safety
    /// The caller must trust the guest image and resume this context on its owning CPU.
    pub(crate) unsafe fn restore(
        &mut self,
    ) -> Result<VirtualMachineExit, VirtualizationError> {
        if self.owner_apic_id != super::interrupts::current_apic_id() {
            return Err(VirtualizationError::HostSetupFailed);
        }
        let _interrupt_guard = InterruptGuard::disable();
        let _host_segments =
            super::segments::initialize().ok_or(VirtualizationError::HostSetupFailed)?;
        let _cr4_guard = Cr4Guard::enable_vmx()?;
        let _session = self.begin_vmx(&self.control_memory, false)?;

        let start_tsc = read_tsc();
        let maximum_guest_cycles = (u128::from(self.controls.tsc_frequency)
            * u128::from(GUEST_TIME_LIMIT_NS)
            / 1_000_000_000)
            .min(u128::from(u64::MAX)) as u64;
        let mut host_fpu = FpuContext::INITIAL;
        // SAFETY: this context owns its VMCS and the VM-exit stub preserves this host call frame.
        let mut entry_status = unsafe {
            if self.launched {
                kz_vmx_resume(&mut self.registers, &mut self.fpu_context, &mut host_fpu)
            } else {
                kz_vmx_launch(&mut self.registers, &mut self.fpu_context, &mut host_fpu)
            }
        };
        loop {
            if entry_status != 0 {
                return Err(vm_instruction_error());
            }
            self.launched = true;
            let mut exit = read_guest_exit(&self.registers)?;
            let next_rip = match exit.reason {
                VirtualMachineExitReason::ExternalInterrupt => {
                    super::interrupts::handle_virtualization_exit(exit.interrupt_info);
                    return Ok(exit);
                }
                VirtualMachineExitReason::Cpuid => {
                    emulate_cpuid(&mut self.registers);
                    Some(exit.guest_rip.checked_add(u64::from(exit.instruction_length)))
                }
                VirtualMachineExitReason::Rdtsc => {
                    let timestamp = read_tsc();
                    self.registers.rax = u64::from(timestamp as u32);
                    self.registers.rdx = u64::from((timestamp >> 32) as u32);
                    Some(exit.guest_rip.checked_add(u64::from(exit.instruction_length)))
                }
                VirtualMachineExitReason::Io => {
                    let Some(next_rip) = emulate_guest_io(&exit, &mut self.registers) else {
                        return Ok(exit);
                    };
                    Some(Some(next_rip))
                }
                VirtualMachineExitReason::EptViolation => {
                    let Some(next_rip) = self.emulate_guest_mmio(&exit) else {
                        return Ok(exit);
                    };
                    Some(Some(next_rip))
                }
                VirtualMachineExitReason::PreemptionTimer => return Ok(exit),
                VirtualMachineExitReason::Vmcall => {
                    if exit.instruction_length == 0 {
                        return Err(VirtualizationError::VmcsSetupFailed);
                    }
                    let Some(next_rip) = exit
                        .guest_rip
                        .checked_add(u64::from(exit.instruction_length))
                    else {
                        return Err(VirtualizationError::VmcsSetupFailed);
                    };
                    if !vmwrite(VMCS_GUEST_RIP, next_rip) {
                        return Err(vm_instruction_error());
                    }
                    exit.guest_rip = next_rip;
                    exit.registers = self.registers;
                    return Ok(exit);
                }
                _ => return Ok(exit),
            };
            let Some(next_rip) = next_rip.flatten() else {
                return Err(VirtualizationError::VmcsSetupFailed);
            };
            if !vmwrite(VMCS_GUEST_RIP, next_rip) {
                return Err(vm_instruction_error());
            }
            exit.guest_rip = next_rip;
            exit.registers = self.registers;
            if read_tsc().wrapping_sub(start_tsc) >= maximum_guest_cycles {
                exit.reason = VirtualMachineExitReason::PreemptionTimer;
                return Ok(exit);
            }
            // SAFETY: the VMCS is launched and this resumes the same guest state.
            entry_status = unsafe {
                kz_vmx_resume(&mut self.registers, &mut self.fpu_context, &mut host_fpu)
            };
        }
    }

    fn begin_vmx(
        &self,
        control_memory: &PhysicalRun,
        clear_vmcs: bool,
    ) -> Result<VmxSession, VirtualizationError> {
        let mut session = VmxSession::new();
        let vmxon_operand = (control_memory.address + VMXON_OFFSET) as u64;
        let vmcs_operand = (control_memory.address + VMCS_OFFSET) as u64;
        let ept_pointer = control_memory.address as u64 | 6 | (3 << 3);
        session.start(&vmxon_operand, &vmcs_operand, ept_pointer, clear_vmcs)?;
        Ok(session)
    }

    fn emulate_guest_mmio(&mut self, exit: &VirtualMachineExit) -> Option<u64> {
        let address = exit.guest_physical_address?;
        if !GuestVirtioDevices::contains(address) {
            return None;
        }
        let access = exit.qualification & 0x7;
        if access != 1 && access != 2 {
            return None;
        }
        let instruction_address = usize::try_from(exit.guest_rip).ok()?;
        let instruction_length = GUEST_MEMORY_SIZE.checked_sub(instruction_address)?.min(15);
        if instruction_length == 0 {
            return None;
        }
        // SAFETY: the stopped guest owns this allocated 2 MiB run; the checked slice stays in it.
        let instruction = unsafe {
            core::slice::from_raw_parts(
                self.guest_memory.pointer_at(instruction_address),
                instruction_length,
            )
        };
        let decoded = decode_mmio_mov(instruction)?;
        if decoded.write != (access == 2) {
            return None;
        }
        let value = if decoded.write {
            if let Some(immediate) = decoded.immediate {
                immediate
            } else {
            read_guest_register(
                &self.registers,
                decoded.register,
                decoded.width,
                decoded.high_byte,
            )?
            }
        } else {
            0
        };
        // SAFETY: the vCPU is stopped at this VM exit and exclusively owns its guest memory.
        let guest_bytes = unsafe {
            core::slice::from_raw_parts_mut(self.guest_memory.pointer(), GUEST_MEMORY_SIZE)
        };
        let mut memory = GuestMemory::new(guest_bytes);
        let value = self.devices.access(
            &mut memory,
            address,
            decoded.width as u8,
            decoded.write,
            value,
        )?;
        if !decoded.write {
            write_guest_register(
                &mut self.registers,
                decoded.register,
                decoded.register_width,
                decoded.high_byte,
                value,
            )?;
        }
        exit.guest_rip.checked_add(decoded.length as u64)
    }
}

struct DecodedMmioMov {
    length: usize,
    register: usize,
    width: usize,
    register_width: usize,
    high_byte: bool,
    immediate: Option<u64>,
    write: bool,
}

fn decode_mmio_mov(instruction: &[u8]) -> Option<DecodedMmioMov> {
    let mut cursor = 0;
    let mut operand_override = false;
    let mut rex = 0;
    while let Some(prefix) = instruction.get(cursor).copied() {
        match prefix {
            0x66 => operand_override = true,
            0x67 | 0xf0 | 0xf2 | 0xf3 | 0x2e | 0x36 | 0x3e | 0x26 | 0x64 | 0x65 => {}
            0x40..=0x4f => rex = prefix,
            _ => break,
        }
        cursor += 1;
        if cursor >= 15 {
            return None;
        }
    }
    let opcode = *instruction.get(cursor)?;
    cursor += 1;
    let mut zero_extend = false;
    let mut immediate_store = false;
    let opcode = if opcode == 0x0f {
        zero_extend = true;
        let second = *instruction.get(cursor)?;
        cursor += 1;
        if !matches!(second, 0xb6 | 0xb7) {
            return None;
        }
        second
    } else if matches!(opcode, 0xc6 | 0xc7) {
        immediate_store = true;
        opcode
    } else {
        opcode
    };
    if !zero_extend && !immediate_store && !matches!(opcode, 0x88 | 0x89 | 0x8a | 0x8b) {
        return None;
    }
    let modrm = *instruction.get(cursor)?;
    cursor += 1;
    let mode = modrm >> 6;
    if mode == 3 {
        return None;
    }
    let register_field = usize::from((modrm >> 3) & 7);
    if immediate_store && register_field != 0 {
        return None;
    }
    let register_extension = if rex & 4 != 0 { 8 } else { 0 };
    let byte_width = matches!(opcode, 0x88 | 0x8a | 0xb6 | 0xc6);
    let high_byte = !zero_extend && byte_width && rex == 0 && (4..=7).contains(&register_field);
    let register = if high_byte {
        register_field - 4
    } else {
        register_field | register_extension
    };
    let base = modrm & 7;
    if mode != 3 && base == 4 {
        let sib = *instruction.get(cursor)?;
        cursor += 1;
        if mode == 0 && sib & 7 == 5 {
            cursor = cursor.checked_add(4)?;
        }
    } else if mode == 0 && base == 5 {
        cursor = cursor.checked_add(4)?;
    }
    if mode == 1 {
        cursor = cursor.checked_add(1)?;
    } else if mode == 2 {
        cursor = cursor.checked_add(4)?;
    }
    let immediate = if immediate_store {
        let immediate_width = if byte_width {
            1
        } else if operand_override {
            2
        } else {
            4
        };
        let immediate_bytes = instruction.get(cursor..cursor.checked_add(immediate_width)?)?;
        cursor += immediate_width;
        let mut bytes = [0; 8];
        bytes[..immediate_width].copy_from_slice(immediate_bytes);
        Some(u64::from_le_bytes(bytes))
    } else {
        None
    };
    if cursor > instruction.len() || cursor > 15 {
        return None;
    }
    Some(DecodedMmioMov {
        length: cursor,
        register,
        width: if byte_width {
            1
        } else if zero_extend {
            2
        } else if rex & 8 != 0 {
            8
        } else if operand_override {
            2
        } else {
            4
        },
        register_width: if immediate_store {
            0
        } else if zero_extend {
            if rex & 8 != 0 {
                8
            } else if operand_override {
                2
            } else {
                4
            }
        } else if rex & 8 != 0 {
            8
        } else if operand_override {
            2
        } else {
            4
        },
        high_byte,
        immediate,
        write: immediate_store || (!zero_extend && matches!(opcode, 0x88 | 0x89)),
    })
}

fn read_guest_register(
    registers: &GuestRegisters,
    index: usize,
    width: usize,
    high_byte: bool,
) -> Option<u64> {
    let value = match index {
        0 => registers.rax,
        1 => registers.rcx,
        2 => registers.rdx,
        3 => registers.rbx,
        4 => vmread(VMCS_GUEST_RSP).ok()?,
        5 => registers.rbp,
        6 => registers.rsi,
        7 => registers.rdi,
        8 => registers.r8,
        9 => registers.r9,
        10 => registers.r10,
        11 => registers.r11,
        12 => registers.r12,
        13 => registers.r13,
        14 => registers.r14,
        15 => registers.r15,
        _ => return None,
    };
    if high_byte {
        return Some((value >> 8) & 0xff);
    }
    Some(match width {
        1 => value & 0xff,
        2 => value & 0xffff,
        4 => value & 0xffff_ffff,
        8 => value,
        _ => return None,
    })
}

fn write_guest_register(
    registers: &mut GuestRegisters,
    index: usize,
    width: usize,
    high_byte: bool,
    value: u64,
) -> Option<()> {
    let current = read_guest_register_full(registers, index)?;
    let updated = if high_byte {
        (current & !(0xff << 8)) | ((value & 0xff) << 8)
    } else {
        match width {
            1 => (current & !0xff) | (value & 0xff),
            2 => (current & !0xffff) | (value & 0xffff),
            4 => value & 0xffff_ffff,
            8 => value,
            _ => return None,
        }
    };
    match index {
        0 => registers.rax = updated,
        1 => registers.rcx = updated,
        2 => registers.rdx = updated,
        3 => registers.rbx = updated,
        4 => return vmwrite(VMCS_GUEST_RSP, updated).then_some(()),
        5 => registers.rbp = updated,
        6 => registers.rsi = updated,
        7 => registers.rdi = updated,
        8 => registers.r8 = updated,
        9 => registers.r9 = updated,
        10 => registers.r10 = updated,
        11 => registers.r11 = updated,
        12 => registers.r12 = updated,
        13 => registers.r13 = updated,
        14 => registers.r14 = updated,
        15 => registers.r15 = updated,
        _ => return None,
    }
    Some(())
}

fn read_guest_register_full(registers: &GuestRegisters, index: usize) -> Option<u64> {
    Some(match index {
        0 => registers.rax,
        1 => registers.rcx,
        2 => registers.rdx,
        3 => registers.rbx,
        4 => vmread(VMCS_GUEST_RSP).ok()?,
        5 => registers.rbp,
        6 => registers.rsi,
        7 => registers.rdi,
        8 => registers.r8,
        9 => registers.r9,
        10 => registers.r10,
        11 => registers.r11,
        12 => registers.r12,
        13 => registers.r13,
        14 => registers.r14,
        15 => registers.r15,
        _ => return None,
    })
}

/// # Safety
/// The caller must trust the ring-0 guest image; firmware and full OS device models are absent.
pub(crate) unsafe fn run_guest(
    image: &[u8],
) -> Result<VirtualMachineExit, VirtualizationError> {
    let mut context = GuestContext::load(image)?;
    unsafe { context.restore() }
}

fn read_guest_exit(
    registers: &GuestRegisters,
) -> Result<VirtualMachineExit, VirtualizationError> {
    let raw_reason = vmread(VMCS_EXIT_REASON)? as u32;
    if raw_reason & (1 << 31) != 0 {
        return Err(VirtualizationError::GuestEntryFailed(raw_reason & 0xffff));
    }
    let reason = raw_reason & 0xffff;
    Ok(VirtualMachineExit {
        reason: match reason {
            0 => VirtualMachineExitReason::ExceptionOrNmi,
            1 => VirtualMachineExitReason::ExternalInterrupt,
            10 => VirtualMachineExitReason::Cpuid,
            12 => VirtualMachineExitReason::Halt,
            16 => VirtualMachineExitReason::Rdtsc,
            18 => VirtualMachineExitReason::Vmcall,
            30 => VirtualMachineExitReason::Io,
            48 => VirtualMachineExitReason::EptViolation,
            52 => VirtualMachineExitReason::PreemptionTimer,
            other => VirtualMachineExitReason::Other(other),
        },
        qualification: vmread(0x6400)?,
        instruction_length: vmread(VMCS_EXIT_INSTRUCTION_LENGTH)? as u32,
        guest_rip: vmread(VMCS_GUEST_RIP)?,
        guest_rsp: vmread(VMCS_GUEST_RSP)?,
        guest_rflags: vmread(VMCS_GUEST_RFLAGS)?,
        guest_physical_address: if reason == 48 {
            Some(vmread(VMCS_GUEST_PHYSICAL_ADDRESS)?)
        } else {
            None
        },
        interrupt_info: vmread(VMCS_EXIT_INTERRUPTION_INFO)? as u32,
        registers: *registers,
    })
}

fn emulate_cpuid(registers: &mut GuestRegisters) {
    let leaf = registers.rax as u32;
    let subleaf = registers.rcx as u32;
    let (eax, mut ebx, mut ecx, mut edx) = if leaf == 0x4000_0000 {
        (
            0x4000_0001,
            u32::from_le_bytes(*b"KZKE"),
            u32::from_le_bytes(*b"RNEL"),
            u32::from_le_bytes(*b"KZVM"),
        )
    } else if (0x4000_0000..=0x4000_00ff).contains(&leaf) {
        (0, 0, 0, 0)
    } else {
        let result = __cpuid_count(leaf, subleaf);
        (result.eax, result.ebx, result.ecx, result.edx)
    };
    if leaf == 1 {
        ebx = (ebx & 0x00ff_ffff) | (1 << 16);
        ecx &= !(
            (1 << 5)
                | (1 << 9)
                | (1 << 12)
                | (1 << 21)
                | (1 << 26)
                | (1 << 27)
                | (1 << 28)
                | (1 << 29)
        );
        ecx |= 1 << 31;
        edx &= !((1 << 9) | (1 << 23));
        if FPU_MODE == 0 {
            edx &= !((1 << 25) | (1 << 26));
        }
    } else if leaf == 7 && subleaf == 0 {
        ebx &= !((1 << 5) | (1 << 16) | (1 << 17) | (1 << 30) | (1 << 31));
        ecx &= !((1 << 9) | (1 << 10));
    } else if leaf == 0x8000_0001 {
        ecx &= !(1 << 2);
        ecx &= !(1 << 27);
    }
    registers.rax = u64::from(eax);
    registers.rbx = u64::from(ebx);
    registers.rcx = u64::from(ecx);
    registers.rdx = u64::from(edx);
}

fn emulate_guest_io(
    exit: &VirtualMachineExit,
    registers: &mut GuestRegisters,
) -> Option<u64> {
    if exit.qualification & ((1 << 4) | (1 << 5)) != 0 {
        return None;
    }
    let width = match exit.qualification & 0x7 {
        0 => 1,
        1 => 2,
        3 => 4,
        _ => return None,
    };
    let input = exit.qualification & (1 << 3) != 0;
    let value = registers.rax as u32;
    let value = crate::kernel::arch::x86_64::guest_devices::emulate_port_io(
        crate::kernel::arch::x86_64::guest_devices::PortIoExit {
            input,
            port: (exit.qualification >> 16) as u16,
            width,
            value,
        },
    )?;
    if input {
        registers.rax = match width {
            1 => (registers.rax & !0xff) | u64::from(value & 0xff),
            2 => (registers.rax & !0xffff) | u64::from(value & 0xffff),
            4 => u64::from(value),
            _ => return None,
        };
    }
    exit.guest_rip.checked_add(u64::from(exit.instruction_length))
}

fn read_tsc() -> u64 {
    let low: u32;
    let high: u32;
    // SAFETY: RDTSC is available in the x86_64 host; the guest has no direct MSR access.
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    (u64::from(high) << 32) | u64::from(low)
}

fn discover_controls() -> Result<VmxControls, VirtualizationError> {
    if __cpuid(1).ecx & (1 << 5) == 0 {
        return Err(VirtualizationError::Unsupported);
    }
    let basic = read_msr(IA32_VMX_BASIC);
    let region_size = ((basic >> 32) & 0x1fff) as usize;
    let memory_type = (basic >> 50) & 0xf;
    if region_size == 0 || region_size > PAGE_SIZE || memory_type != 6 {
        return Err(VirtualizationError::Unsupported);
    }
    let use_true_controls = basic & (1 << 55) != 0;
    let pin_msr = if use_true_controls {
        IA32_VMX_TRUE_PINBASED_CTLS
    } else {
        IA32_VMX_PINBASED_CTLS
    };
    let processor_msr = if use_true_controls {
        IA32_VMX_TRUE_PROCBASED_CTLS
    } else {
        IA32_VMX_PROCBASED_CTLS
    };
    let exit_msr = if use_true_controls {
        IA32_VMX_TRUE_EXIT_CTLS
    } else {
        IA32_VMX_EXIT_CTLS
    };
    let entry_msr = if use_true_controls {
        IA32_VMX_TRUE_ENTRY_CTLS
    } else {
        IA32_VMX_ENTRY_CTLS
    };
    let pin_based = adjust_controls((1 << 0) | (1 << 3) | (1 << 6), read_msr(pin_msr))?;
    let primary = adjust_controls(
        (1 << 7)
            | (1 << 10)
            | (1 << 11)
            | (1 << 12)
            | (1 << 19)
            | (1 << 20)
            | (1 << 23)
            | (1 << 24)
            | (1 << 28)
            | (1 << 29)
            | (1 << 31),
        read_msr(processor_msr),
    )?;
    let secondary = adjust_controls(1 << 1, read_msr(IA32_VMX_PROCBASED_CTLS2))?;
    let exit = adjust_controls((1 << 9) | (1 << 21), read_msr(exit_msr))?;
    let entry = adjust_controls((1 << 9) | (1 << 15), read_msr(entry_msr))?;
    let ept_vpid = read_msr(IA32_VMX_EPT_VPID_CAP);
    let required_ept = (1 << 6) | (1 << 14) | (1 << 16) | (1 << 20) | (1 << 25);
    if ept_vpid & required_ept != required_ept {
        return Err(VirtualizationError::Unsupported);
    }
    let tsc_frequency = invariant_tsc_frequency().ok_or(VirtualizationError::Unsupported)?;
    let timer_rate = (read_msr(IA32_VMX_MISC) & 0x1f) as u32;
    let timer_divisor = 1u128 << timer_rate;
    let timer_ticks = u128::from(tsc_frequency) * u128::from(GUEST_TIME_LIMIT_NS)
        / (1_000_000_000 * timer_divisor);
    if timer_ticks == 0 {
        return Err(VirtualizationError::Unsupported);
    }
    let timer_ticks = timer_ticks.min(u128::from(u32::MAX)) as u32;
    Ok(VmxControls {
        pin_based,
        primary,
        secondary,
        exit,
        entry,
        revision: basic as u32 & 0x7fff_ffff,
        preemption_timer: timer_ticks,
        tsc_frequency,
        physical_address_limit: MemoryArchitecture::physical_address_limit(),
        revision_requires_32bit_address: basic & (1 << 48) != 0,
    })
}

fn invariant_tsc_frequency() -> Option<u64> {
    let maximum_basic_leaf = __cpuid(0).eax;
    if maximum_basic_leaf < 0x15 {
        return None;
    }
    let tsc_ratio = __cpuid(0x15);
    if tsc_ratio.eax == 0 || tsc_ratio.ebx == 0 || tsc_ratio.ecx == 0 {
        return None;
    }
    let maximum_extended_leaf = __cpuid(0x8000_0000).eax;
    if maximum_extended_leaf < 0x8000_0007
        || __cpuid(0x8000_0007).edx & (1 << 8) == 0
    {
        return None;
    }
    let frequency = u128::from(tsc_ratio.ecx) * u128::from(tsc_ratio.ebx)
        / u128::from(tsc_ratio.eax);
    u64::try_from(frequency).ok().filter(|frequency| *frequency != 0)
}

fn adjust_controls(requested: u32, capability: u64) -> Result<u32, VirtualizationError> {
    let required_one = !(capability as u32);
    let allowed_one = (capability >> 32) as u32;
    let controls = requested | required_one;
    if controls & !allowed_one != 0 {
        return Err(VirtualizationError::Unsupported);
    }
    Ok(controls)
}

fn validate_physical_addresses(
    controls: &VmxControls,
    guest_memory: &PhysicalRun,
    control_memory: &PhysicalRun,
) -> Result<(), VirtualizationError> {
    let end = control_memory
        .address
        .checked_add(control_memory.length)
        .ok_or(VirtualizationError::Unsupported)?;
    let guest_end = guest_memory
        .address
        .checked_add(guest_memory.length)
        .ok_or(VirtualizationError::Unsupported)?;
    if end as u64 > controls.physical_address_limit
        || guest_end as u64 > controls.physical_address_limit
        || (controls.revision_requires_32bit_address
            && (end as u64 > (1u64 << 32) || guest_end as u64 > (1u64 << 32)))
    {
        return Err(VirtualizationError::Unsupported);
    }
    Ok(())
}

fn initialize_guest_memory(guest_memory: &PhysicalRun, image: &[u8]) {
    guest_memory.zero();
    let tss_base = GUEST_TSS as u64;
    let tss_limit = 0x67u64;
    let tss_descriptor = (tss_limit & 0xffff)
        | ((tss_base & 0xffff) << 16)
        | (((tss_base >> 16) & 0xff) << 32)
        | (0x8b << 40)
        | (((tss_limit >> 16) & 0xf) << 48)
        | (((tss_base >> 24) & 0xff) << 56);
    // SAFETY: guest memory is a disjoint allocator-owned run; the image fits its checked range.
    unsafe {
        copy_nonoverlapping(
            image.as_ptr(),
            guest_memory.pointer().add(GUEST_ENTRY),
            image.len(),
        );
        write_volatile(guest_memory.pointer_at(PAGE_SIZE).cast::<u64>(), 0x2000 | 0x3);
        write_volatile(
            guest_memory.pointer_at(2 * PAGE_SIZE).cast::<u64>(),
            0x3000 | 0x3,
        );
        write_volatile(
            guest_memory.pointer_at(3 * PAGE_SIZE).cast::<u64>(),
            0x83,
        );
        write_volatile(
            guest_memory
                .pointer_at(3 * PAGE_SIZE)
                .cast::<u64>()
                .add(1),
            GUEST_MEMORY_SIZE as u64 | 0x83,
        );
        write_volatile(
            guest_memory
                .pointer_at(3 * PAGE_SIZE)
                .cast::<u64>()
                .add((VirtioConsole::MMIO_BASE as usize) / (2 * 1024 * 1024)),
            VirtioConsole::MMIO_BASE | 0x83,
        );
        let guest_gdt = guest_memory.pointer_at(GUEST_GDT).cast::<u64>();
        write_volatile(guest_gdt, 0);
        write_volatile(guest_gdt.add(1), 0x00af_9a00_0000_ffff);
        write_volatile(guest_gdt.add(2), 0x00cf_9200_0000_ffff);
        write_volatile(guest_gdt.add(3), tss_descriptor);
        write_volatile(guest_gdt.add(4), tss_base >> 32);
        write_volatile(
            guest_memory.pointer_at(GUEST_TSS + 102).cast::<u16>(),
            0x68,
        );
    }
}

fn initialize_control_memory(
    control_memory: &PhysicalRun,
    guest_memory: &PhysicalRun,
    controls: &VmxControls,
) {
    control_memory.zero();
    // SAFETY: all pointers refer to aligned pages in the allocated identity-mapped control run.
    unsafe {
        write_volatile(
            control_memory.pointer_at(EPT_PML4_OFFSET).cast::<u64>(),
            (control_memory.address + EPT_PDPT_OFFSET) as u64 | 0x7,
        );
        write_volatile(
            control_memory.pointer_at(EPT_PDPT_OFFSET).cast::<u64>(),
            (control_memory.address + EPT_PD_OFFSET) as u64 | 0x7,
        );
        write_volatile(
            control_memory.pointer_at(EPT_PD_OFFSET).cast::<u64>(),
            guest_memory.address as u64 | 0xb7,
        );
        write_volatile(
            control_memory
                .pointer_at(EPT_PD_OFFSET)
                .cast::<u64>()
                .add(1),
            (control_memory.address + EPT_DEVICE_PT_OFFSET) as u64 | 0x7,
        );
        write_bytes(
            control_memory.pointer_at(MSR_BITMAP_OFFSET),
            0xff,
            PAGE_SIZE,
        );
        write_volatile(
            control_memory.pointer_at(VMXON_OFFSET).cast::<u32>(),
            controls.revision,
        );
        write_volatile(
            control_memory.pointer_at(VMCS_OFFSET).cast::<u32>(),
            controls.revision,
        );
        write_msr_entry(
            control_memory.pointer_at(ENTRY_MSR_LIST_OFFSET),
            IA32_KERNEL_GS_BASE,
            0,
        );
        write_msr_entry(
            control_memory.pointer_at(EXIT_STORE_MSR_LIST_OFFSET),
            IA32_KERNEL_GS_BASE,
            0,
        );
        write_msr_entry(
            control_memory.pointer_at(EXIT_LOAD_MSR_LIST_OFFSET),
            IA32_KERNEL_GS_BASE,
            read_msr(IA32_KERNEL_GS_BASE),
        );
    }
}

unsafe fn write_msr_entry(address: *mut u8, index: u32, value: u64) {
    let entry = address.cast::<VmxMsrEntry>();
    unsafe {
        write_volatile(
            entry,
            VmxMsrEntry {
                index,
                reserved: 0,
                value,
            },
        );
    }
}

fn configure_vmcs(
    controls: &VmxControls,
    control_memory: &PhysicalRun,
    host_segments: super::segments::HostSegmentState,
) -> Result<(), VirtualizationError> {
    let guest_cr0 = fixed_guest_value(
        CR0_PE | CR0_NE | CR0_WP | CR0_PG,
        read_msr(IA32_VMX_CR0_FIXED0),
        read_msr(IA32_VMX_CR0_FIXED1),
    )?;
    let guest_fp_cr4 = if FPU_MODE == 0 {
        0
    } else {
        CR4_OSFXSR | CR4_OSXMMEXCPT
    };
    let guest_cr4 = fixed_guest_value(
        CR4_PAE | guest_fp_cr4,
        read_msr(IA32_VMX_CR4_FIXED0),
        read_msr(IA32_VMX_CR4_FIXED1),
    )?;
    let unsupported_guest_cr4 = CR4_LA57 | CR4_OSXSAVE | CR4_PKE | CR4_CET | CR4_PKS;
    if guest_cr0 & (CR0_PE | CR0_PG) != (CR0_PE | CR0_PG)
        || guest_cr0 & (CR0_EM | CR0_TS) != 0
        || guest_cr4 & CR4_PAE == 0
        || guest_cr4 & guest_fp_cr4 != guest_fp_cr4
        || guest_cr4 & unsupported_guest_cr4 != 0
    {
        return Err(VirtualizationError::Unsupported);
    }
    let efer = read_msr(IA32_EFER);
    let guest_efer = (efer | EFER_LME | EFER_LMA) & (EFER_LME | EFER_LMA | EFER_NXE);
    let ept_pointer = control_memory.address as u64
        | EPT_PML4_OFFSET as u64
        | 6
        | (3 << 3);
    let mut host_rsp: u64;
    // SAFETY: reading the current stack pointer is valid in this x86_64 kernel context.
    unsafe {
        core::arch::asm!(
            "mov {value}, rsp",
            value = out(reg) host_rsp,
            options(nomem, nostack, preserves_flags),
        );
    }
    let cr0 = read_cr0();
    let cr3 = read_cr3();
    let cr4 = read_cr4();
    let cr0_mask = u64::from(u32::MAX);
    let cr4_mask = u64::from(u32::MAX);
    let guest_fields = [
        (VMCS_PIN_BASED_CONTROLS, u64::from(controls.pin_based)),
        (VMCS_PRIMARY_PROCESSOR_CONTROLS, u64::from(controls.primary)),
        (VMCS_EXCEPTION_BITMAP, u64::from(u32::MAX)),
        (
            VMCS_PREEMPTION_TIMER_VALUE,
            u64::from(controls.preemption_timer),
        ),
        (VMCS_VM_EXIT_CONTROLS, u64::from(controls.exit)),
        (VMCS_VM_ENTRY_CONTROLS, u64::from(controls.entry)),
        (VMCS_GUEST_VMCS_LINK_POINTER, u64::MAX),
        (
            VMCS_SECONDARY_PROCESSOR_CONTROLS,
            u64::from(controls.secondary),
        ),
        (VMCS_VM_EXIT_MSR_STORE_COUNT, 1),
        (VMCS_VM_EXIT_MSR_LOAD_COUNT, 1),
        (VMCS_VM_ENTRY_MSR_LOAD_COUNT, 1),
        (
            VMCS_MSR_BITMAP,
            (control_memory.address + MSR_BITMAP_OFFSET) as u64,
        ),
        (
            VMCS_VM_EXIT_MSR_STORE_ADDRESS,
            (control_memory.address + EXIT_STORE_MSR_LIST_OFFSET) as u64,
        ),
        (
            VMCS_VM_EXIT_MSR_LOAD_ADDRESS,
            (control_memory.address + EXIT_LOAD_MSR_LIST_OFFSET) as u64,
        ),
        (
            VMCS_VM_ENTRY_MSR_LOAD_ADDRESS,
            (control_memory.address + ENTRY_MSR_LIST_OFFSET) as u64,
        ),
        (VMCS_EPT_POINTER, ept_pointer),
        (0x6000, cr0_mask),
        (0x6002, cr4_mask),
        (0x6004, guest_cr0),
        (0x6006, guest_cr4),
        (VMCS_GUEST_ES_SELECTOR, 0x10),
        (VMCS_GUEST_CS_SELECTOR, 0x08),
        (VMCS_GUEST_SS_SELECTOR, 0x10),
        (VMCS_GUEST_DS_SELECTOR, 0x10),
        (VMCS_GUEST_FS_SELECTOR, 0),
        (VMCS_GUEST_GS_SELECTOR, 0),
        (VMCS_GUEST_LDTR_SELECTOR, 0),
        (VMCS_GUEST_TR_SELECTOR, 0x18),
        (VMCS_GUEST_ES_LIMIT, u64::from(u32::MAX)),
        (VMCS_GUEST_CS_LIMIT, u64::from(u32::MAX)),
        (VMCS_GUEST_SS_LIMIT, u64::from(u32::MAX)),
        (VMCS_GUEST_DS_LIMIT, u64::from(u32::MAX)),
        (VMCS_GUEST_FS_LIMIT, 0),
        (VMCS_GUEST_GS_LIMIT, 0),
        (VMCS_GUEST_LDTR_LIMIT, 0),
        (VMCS_GUEST_TR_LIMIT, 0x67),
        (VMCS_GUEST_GDTR_LIMIT, (5 * size_of::<u64>() - 1) as u64),
        (VMCS_GUEST_IDTR_LIMIT, 0),
        (VMCS_GUEST_ES_ACCESS_RIGHTS, 0xc093),
        (VMCS_GUEST_CS_ACCESS_RIGHTS, 0xa09b),
        (VMCS_GUEST_SS_ACCESS_RIGHTS, 0xc093),
        (VMCS_GUEST_DS_ACCESS_RIGHTS, 0xc093),
        (VMCS_GUEST_FS_ACCESS_RIGHTS, 1 << 16),
        (VMCS_GUEST_GS_ACCESS_RIGHTS, 1 << 16),
        (VMCS_GUEST_LDTR_ACCESS_RIGHTS, 1 << 16),
        (VMCS_GUEST_TR_ACCESS_RIGHTS, 0x8b),
        (VMCS_GUEST_INTERRUPTIBILITY, 0),
        (VMCS_GUEST_ACTIVITY_STATE, 0),
        (VMCS_GUEST_PENDING_DEBUG_EXCEPTIONS, 0),
        (VMCS_GUEST_SYSENTER_CS, 0),
        (VMCS_GUEST_CR0, guest_cr0),
        (VMCS_GUEST_CR3, 0x1000),
        (VMCS_GUEST_CR4, guest_cr4),
        (VMCS_GUEST_ES_BASE, 0),
        (VMCS_GUEST_CS_BASE, 0),
        (VMCS_GUEST_SS_BASE, 0),
        (VMCS_GUEST_DS_BASE, 0),
        (VMCS_GUEST_FS_BASE, 0),
        (VMCS_GUEST_GS_BASE, 0),
        (VMCS_GUEST_LDTR_BASE, 0),
        (VMCS_GUEST_TR_BASE, GUEST_TSS as u64),
        (VMCS_GUEST_GDTR_BASE, GUEST_GDT as u64),
        (VMCS_GUEST_IDTR_BASE, 0),
        (VMCS_GUEST_DR7, 0x400),
        (VMCS_GUEST_RSP, GUEST_STACK),
        (VMCS_GUEST_RIP, GUEST_ENTRY as u64),
        (VMCS_GUEST_RFLAGS, 2),
        (VMCS_GUEST_SYSENTER_ESP, 0),
        (VMCS_GUEST_SYSENTER_EIP, 0),
        (VMCS_GUEST_IA32_EFER, guest_efer),
        (VMCS_HOST_SYSENTER_CS, read_msr(IA32_SYSENTER_CS)),
        (VMCS_HOST_CR0, cr0),
        (VMCS_HOST_CR3, cr3),
        (VMCS_HOST_CR4, cr4),
        (VMCS_HOST_FS_BASE, read_msr(IA32_FS_BASE)),
        (VMCS_HOST_GS_BASE, read_msr(IA32_GS_BASE)),
        (VMCS_HOST_TR_BASE, host_segments.tss_base),
        (VMCS_HOST_GDTR_BASE, host_segments.gdt_base),
        (VMCS_HOST_IDTR_BASE, super::interrupts::idt_base()),
        (VMCS_HOST_SYSENTER_ESP, read_msr(IA32_SYSENTER_ESP)),
        (VMCS_HOST_SYSENTER_EIP, read_msr(IA32_SYSENTER_EIP)),
        (VMCS_HOST_RSP, host_rsp),
        (VMCS_HOST_RIP, kz_vmx_exit as *const () as u64),
        (VMCS_HOST_IA32_EFER, efer),
        (VMCS_HOST_ES_SELECTOR, 0x10),
        (VMCS_HOST_CS_SELECTOR, 0x08),
        (VMCS_HOST_SS_SELECTOR, 0x10),
        (VMCS_HOST_DS_SELECTOR, 0x10),
        (VMCS_HOST_FS_SELECTOR, 0),
        (VMCS_HOST_GS_SELECTOR, 0),
        (
            VMCS_HOST_TR_SELECTOR,
            u64::from(host_segments.tss_selector),
        ),
    ];
    for (field, value) in guest_fields {
        if !vmwrite(field, value) {
            return Err(vm_instruction_error());
        }
    }
    Ok(())
}

fn fixed_guest_value(
    requested: u64,
    required_one: u64,
    allowed_one: u64,
) -> Result<u64, VirtualizationError> {
    if required_one & !allowed_one != 0 {
        return Err(VirtualizationError::Unsupported);
    }
    let value = (requested | required_one) & allowed_one;
    Ok(value)
}

fn satisfies_fixed_bits(value: u64, required_one: u64, allowed_one: u64) -> bool {
    value & required_one == required_one && value & !allowed_one == 0
}

fn enable_vmx_feature_control() -> Result<(), VirtualizationError> {
    let feature_control = read_msr(IA32_FEATURE_CONTROL);
    if feature_control & 1 == 0 {
        #[cfg(feature = "intel-vtx-lock-feature-control")]
        {
            // SAFETY: this explicitly enabled feature permanently locks VMX outside SMX on this CPU.
            unsafe { write_msr(IA32_FEATURE_CONTROL, feature_control | 1 | (1 << 2)) };
        }
        #[cfg(not(feature = "intel-vtx-lock-feature-control"))]
        return Err(VirtualizationError::VmxDisabledByFirmware);
    }
    let feature_control = read_msr(IA32_FEATURE_CONTROL);
    if feature_control & (1 | (1 << 2)) != (1 | (1 << 2)) {
        return Err(VirtualizationError::VmxDisabledByFirmware);
    }
    Ok(())
}

fn vm_instruction_error() -> VirtualizationError {
    VirtualizationError::VmxInstructionFailed(vmread(VMCS_VM_INSTRUCTION_ERROR).unwrap_or(u64::MAX) as u32)
}

fn vmread(field: u64) -> Result<u64, VirtualizationError> {
    let value: u64;
    let failed: u8;
    // SAFETY: this runs in VMX root operation with the current VMCS loaded.
    unsafe {
        core::arch::asm!(
            "vmread {value}, {field}",
            "setbe {failed}",
            value = out(reg) value,
            field = in(reg) field,
            failed = out(reg_byte) failed,
            options(nostack),
        );
    }
    if failed == 0 {
        Ok(value)
    } else {
        Err(VirtualizationError::VmcsSetupFailed)
    }
}

fn vmwrite(field: u64, value: u64) -> bool {
    let failed: u8;
    // SAFETY: this runs in VMX root operation with the current VMCS loaded.
    unsafe {
        core::arch::asm!(
            "vmwrite {value}, {field}",
            "setbe {failed}",
            value = in(reg) value,
            field = in(reg) field,
            failed = out(reg_byte) failed,
            options(nostack),
        );
    }
    failed == 0
}

macro_rules! define_vmx_memory_instruction {
    ($name:ident, $instruction:literal) => {
        fn $name(pointer: *const u64) -> bool {
            let failed: u8;
            // SAFETY: `pointer` addresses an identity-mapped VMX operand in allocated RAM.
            unsafe {
                core::arch::asm!(
                    $instruction,
                    "setbe {failed}",
                    pointer = in(reg) pointer,
                    failed = lateout(reg_byte) failed,
                    options(nostack),
                );
            }
            failed == 0
        }
    };
}

define_vmx_memory_instruction!(vmxon, "vmxon [{pointer}]");
define_vmx_memory_instruction!(vmclear, "vmclear [{pointer}]");
define_vmx_memory_instruction!(vmptrld, "vmptrld [{pointer}]");

unsafe fn invept_single_context(ept_pointer: u64) -> bool {
    let descriptor = EptInvalidationDescriptor {
        ept_pointer,
        reserved: 0,
    };
    let failed: u8;
    // SAFETY: VMX operation is active and discovery checked single-context INVEPT support.
    unsafe {
        core::arch::asm!(
            "invept {kind}, [{descriptor}]",
            "setbe {failed}",
            kind = in(reg) 1u64,
            descriptor = in(reg) &descriptor,
            failed = lateout(reg_byte) failed,
            options(nostack),
        );
    }
    failed == 0
}

unsafe fn vmxoff() -> bool {
    let failed: u8;
    // SAFETY: called only while this CPU is in VMX root operation.
    unsafe {
        core::arch::asm!(
            "vmxoff",
            "setbe {failed}",
            failed = lateout(reg_byte) failed,
            options(nostack),
        );
    }
    failed == 0
}

fn read_msr(index: u32) -> u64 {
    let low: u32;
    let high: u32;
    // SAFETY: all call sites use architectural MSRs supported by the checked x86_64 host.
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") index,
            out("eax") low,
            out("edx") high,
            options(nostack, preserves_flags),
        );
    }
    (u64::from(high) << 32) | u64::from(low)
}

unsafe fn write_msr(index: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
    // SAFETY: the caller verifies the MSR is writable before changing it.
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") index,
            in("eax") low,
            in("edx") high,
            options(nostack, preserves_flags),
        );
    }
}

fn read_cr0() -> u64 {
    let value: u64;
    // SAFETY: CR0 is readable at the kernel's current privilege level.
    unsafe {
        core::arch::asm!(
            "mov {value}, cr0",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

fn read_cr3() -> u64 {
    let value: u64;
    // SAFETY: CR3 is readable at the kernel's current privilege level.
    unsafe {
        core::arch::asm!(
            "mov {value}, cr3",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

fn read_cr4() -> u64 {
    let value: u64;
    // SAFETY: CR4 is readable at the kernel's current privilege level.
    unsafe {
        core::arch::asm!(
            "mov {value}, cr4",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

unsafe fn write_cr4(value: u64) {
    // SAFETY: callers validate the VMX fixed-bit masks before writing CR4.
    unsafe {
        core::arch::asm!(
            "mov cr4, {value}",
            value = in(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
}