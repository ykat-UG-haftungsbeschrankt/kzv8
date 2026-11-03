/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::mem::size_of;
use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, Ordering};

static PAGE_FAULT_DIAGNOSTIC: AtomicBool = AtomicBool::new(false);
const FPU_MODE: usize = if cfg!(feature = "fpu-x87") {
    0
} else if cfg!(feature = "fpu-avx") {
    2
} else {
    1
};

core::arch::global_asm!(
    ".section .text,\"ax\"",
    ".global x86_unhandled_interrupt",
    "x86_unhandled_interrupt:",
    "cli",
    "1:",
    "hlt",
    "jmp 1b",
    ".global x86_irq_entry",
    "x86_irq_entry:",
    "cld",
    "push rax",
    "push rbx",
    "push rcx",
    "push rdx",
    "push rbp",
    "push rsi",
    "push rdi",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov r12, rsp",
    ".if {fpu_mode} == 0",
    "sub rsp, 128",
    "and rsp, -16",
    "fnsave [rsp]",
    ".elseif {fpu_mode} == 1",
    "sub rsp, 528",
    "and rsp, -16",
    "fxsave64 [rsp]",
    ".else",
    "sub rsp, 896",
    "and rsp, -64",
    "mov r8, rsp",
    "mov eax, 7",
    "xor edx, edx",
    "xsave64 [r8]",
    ".endif",
    "call x86_irq_dispatch",
    ".if {fpu_mode} == 0",
    "frstor [rsp]",
    ".elseif {fpu_mode} == 1",
    "fxrstor64 [rsp]",
    ".else",
    "mov r8, rsp",
    "mov eax, 7",
    "xor edx, edx",
    "xrstor64 [r8]",
    ".endif",
    "mov rsp, r12",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rdi",
    "pop rsi",
    "pop rbp",
    "pop rdx",
    "pop rcx",
    "pop rbx",
    "pop rax",
    "iretq",
    ".global x86_page_fault_entry",
    "x86_page_fault_entry:",
    "cld",
    "push rax",
    "push rbx",
    "push rcx",
    "push rdx",
    "push rbp",
    "push rsi",
    "push rdi",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov r12, rsp",
    ".if {fpu_mode} == 0",
    "sub rsp, 128",
    "and rsp, -16",
    "fnsave [rsp]",
    ".elseif {fpu_mode} == 1",
    "sub rsp, 528",
    "and rsp, -16",
    "fxsave64 [rsp]",
    ".else",
    "sub rsp, 896",
    "and rsp, -64",
    "mov r8, rsp",
    "mov eax, 7",
    "xor edx, edx",
    "xsave64 [r8]",
    ".endif",
    "mov rdi, cr2",
    "mov rsi, [r12 + 120]",
    "call x86_page_fault_dispatch",
    ".if {fpu_mode} == 0",
    "frstor [rsp]",
    ".elseif {fpu_mode} == 1",
    "fxrstor64 [rsp]",
    ".else",
    "mov r8, rsp",
    "mov eax, 7",
    "xor edx, edx",
    "xrstor64 [r8]",
    ".endif",
    "mov rsp, r12",
    "test rax, rax",
    "jz x86_unhandled_interrupt",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rdi",
    "pop rsi",
    "pop rbp",
    "pop rdx",
    "pop rcx",
    "pop rbx",
    "pop rax",
    "add rsp, 8",
    "iretq",
    ".global x86_spurious_interrupt",
    "x86_spurious_interrupt:",
    "iretq",
    fpu_mode = const FPU_MODE,
);

unsafe extern "C" {
    fn x86_unhandled_interrupt();
    fn x86_irq_entry();
    fn x86_page_fault_entry();
    fn x86_spurious_interrupt();
}

const VIRTIO_VECTOR: usize = 0x40;
const EXECUTOR_IPI_VECTOR: usize = 0x41;
const TIMER_VECTOR: usize = 0x42;
const SPURIOUS_VECTOR: usize = 0xff;
const LOCAL_APIC_BASE: usize = 0xfee0_0000;
const IO_APIC_BASE: usize = 0xfec0_0000;
const APIC_BASE_MSR: u32 = 0x1b;
const APIC_ENABLE: u64 = 1 << 11;
const X2APIC_ENABLE: u64 = 1 << 10;
const VIRTIO_GSI_BASE: usize = 5;

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_middle: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const fn interrupt(handler: usize) -> Self {
        let address = handler as u64;
        Self {
            offset_low: address as u16,
            selector: 0x08,
            ist: 0,
            attributes: 0x8e,
            offset_middle: (address >> 16) as u16,
            offset_high: (address >> 32) as u32,
            reserved: 0,
        }
    }
}

#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u64,
}

static mut IDT: [IdtEntry; 256] = [IdtEntry::interrupt(0); 256];

pub(crate) unsafe fn initialize() {
    let handler = x86_unhandled_interrupt as *const () as usize;
    let entries = addr_of_mut!(IDT).cast::<IdtEntry>();
    for index in 0..256 {
        entries.add(index).write(IdtEntry::interrupt(handler));
    }
    entries
        .add(VIRTIO_VECTOR)
        .write(IdtEntry::interrupt(x86_irq_entry as *const () as usize));
    entries
        .add(EXECUTOR_IPI_VECTOR)
        .write(IdtEntry::interrupt(x86_irq_entry as *const () as usize));
    entries
        .add(TIMER_VECTOR)
        .write(IdtEntry::interrupt(x86_irq_entry as *const () as usize));
    entries.add(14).write(IdtEntry::interrupt(
        x86_page_fault_entry as *const () as usize,
    ));
    entries.add(SPURIOUS_VECTOR).write(IdtEntry::interrupt(
        x86_spurious_interrupt as *const () as usize,
    ));
    load_idt();
}

pub(crate) unsafe fn initialize_secondary() {
    load_idt();
}

unsafe fn load_idt() {
    let pointer = IdtPointer {
        limit: (size_of::<[IdtEntry; 256]>() - 1) as u16,
        base: addr_of!(IDT) as u64,
    };
    core::arch::asm!(
        "lidt [{pointer}]",
        pointer = in(reg) &pointer,
        options(readonly, nostack, preserves_flags),
    );
}

pub(crate) fn initialize_controller() -> bool {
    if !initialize_local_controller() {
        return false;
    }

    let apic_id = read_local_apic(0x20) & 0xff00_0000;
    let maximum_irq = (read_io_apic(1) >> 16) & 0xff;
    if maximum_irq < VIRTIO_GSI_BASE as u32 {
        return false;
    }
    for irq in 0..=maximum_irq as usize {
        let redirection = 0x10 + (irq * 2) as u8;
        write_io_apic(redirection + 1, apic_id);
        write_io_apic(redirection, (1 << 16) | VIRTIO_VECTOR as u32);
    }

    let window = crate::kernel::arch::virtio_window();
    let first_unrouted_irq = maximum_irq as usize + 1;
    let last_virtio_irq = (VIRTIO_GSI_BASE + window.slots).min(first_unrouted_irq);
    for irq in VIRTIO_GSI_BASE..last_virtio_irq {
        let redirection = 0x10 + (irq * 2) as u8;
        write_io_apic(redirection, VIRTIO_VECTOR as u32);
    }
    true
}

pub(crate) fn initialize_local_controller() -> bool {
    let apic_base = read_msr(APIC_BASE_MSR);
    if apic_base & X2APIC_ENABLE != 0 {
        return false;
    }
    write_msr(APIC_BASE_MSR, apic_base | APIC_ENABLE);

    write_local_apic(0x80, 0);
    write_local_apic(0xf0, (1 << 8) | SPURIOUS_VECTOR as u32);
    true
}

pub(crate) fn begin_timer_calibration() {
    write_local_apic(0x3e0, 0x3);
    write_local_apic(0x320, TIMER_VECTOR as u32 | (1 << 16));
    write_local_apic(0x380, u32::MAX);
}

pub(crate) fn local_timer_current_count() -> u32 {
    read_local_apic(0x390)
}

pub(crate) fn initialize_timer(initial_count: u32) {
    write_local_apic(0x3e0, 0x3);
    write_local_apic(0x320, TIMER_VECTOR as u32 | (1 << 17));
    write_local_apic(0x380, initial_count);
}

#[no_mangle]
pub extern "C" fn x86_irq_dispatch() {
    let in_service = read_local_apic(0x100 + (EXECUTOR_IPI_VECTOR / 32) * 0x10);
    if in_service & (1 << (TIMER_VECTOR % 32)) != 0 {
        write_local_apic(0xb0, 0);
        crate::kernel::scheduler::thread::ThreadManager::timer_tick();
        return;
    }
    if in_service & (1 << (EXECUTOR_IPI_VECTOR % 32)) == 0 {
        crate::kz_handle_virtio_interrupt();
    }
    write_local_apic(0xb0, 0);
}

pub(crate) fn send_ipi(destination: u8, command: u32) {
    write_local_apic(0x310, u32::from(destination) << 24);
    write_local_apic(0x300, command);
}

pub(crate) fn ipi_pending() -> bool {
    read_local_apic(0x300) & (1 << 12) != 0
}

fn read_msr(index: u32) -> u64 {
    let low: u32;
    let high: u32;
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

fn write_msr(index: u32, value: u64) {
    let low = value as u32;
    let high = (value >> 32) as u32;
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

fn read_local_apic(offset: usize) -> u32 {
    unsafe { core::ptr::read_volatile((LOCAL_APIC_BASE + offset) as *const u32) }
}

fn write_local_apic(offset: usize, value: u32) {
    unsafe { core::ptr::write_volatile((LOCAL_APIC_BASE + offset) as *mut u32, value) }
}

fn read_io_apic(index: u8) -> u32 {
    unsafe {
        core::ptr::write_volatile(IO_APIC_BASE as *mut u32, u32::from(index));
        core::ptr::read_volatile((IO_APIC_BASE + 0x10) as *const u32)
    }
}

fn write_io_apic(index: u8, value: u32) {
    unsafe {
        core::ptr::write_volatile(IO_APIC_BASE as *mut u32, u32::from(index));
        core::ptr::write_volatile((IO_APIC_BASE + 0x10) as *mut u32, value);
    }
}

pub(crate) fn disable() -> u64 {
    let flags: u64;
    unsafe {
        core::arch::asm!(
            "pushfq",
            "pop {flags}",
            "cli",
            flags = out(reg) flags,
            options(preserves_flags),
        );
    }
    flags
}

pub(crate) fn enable() {
    unsafe {
        core::arch::asm!("sti", options(nomem, nostack));
    }
}

pub(crate) fn restore(flags: u64) {
    if flags & (1 << 9) != 0 {
        enable();
    } else {
        let _ = disable();
    }
}

pub(crate) fn enabled() -> bool {
    let flags: u64;
    unsafe {
        core::arch::asm!(
            "pushfq",
            "pop {flags}",
            flags = out(reg) flags,
            options(preserves_flags),
        );
    }
    flags & (1 << 9) != 0
}

#[no_mangle]
extern "C" fn x86_page_fault_dispatch(address: usize, error_code: u64) -> u64 {
    const REJECTED_FAULT_BITS: u64 = (1 << 0) | (1 << 2) | (1 << 3) | (1 << 5) | (1 << 6);
    if error_code & REJECTED_FAULT_BITS != 0 {
        return 0;
    }
    let access = if error_code & (1 << 4) != 0 {
        crate::kernel::memory::r#virtual::FaultAccess::Execute
    } else if error_code & (1 << 1) != 0 {
        crate::kernel::memory::r#virtual::FaultAccess::Write
    } else {
        crate::kernel::memory::r#virtual::FaultAccess::Read
    };
    let resolved = crate::kernel::memory::r#virtual::resolve_page_fault(address, access);
    if !PAGE_FAULT_DIAGNOSTIC.swap(true, Ordering::AcqRel) {
        let message = if resolved {
            b"kz: first lazy page fault resolved\n" as &[u8]
        } else {
            b"kz: first lazy page fault rejected\n" as &[u8]
        };
        let _written = crate::kernel::drivers::virtio::console::Console::write(message);
    }
    resolved as u64
}
