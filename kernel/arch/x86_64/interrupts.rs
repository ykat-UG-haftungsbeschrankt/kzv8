use core::mem::size_of;
use core::ptr::{addr_of, addr_of_mut};

core::arch::global_asm!(
    ".section .text,\"ax\"",
    ".global x86_unhandled_interrupt",
    "x86_unhandled_interrupt:",
    "cli",
    "1:",
    "hlt",
    "jmp 1b",
);

unsafe extern "C" {
    fn x86_unhandled_interrupt();
}

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
    let handler = x86_unhandled_interrupt as usize;
    let entries = addr_of_mut!(IDT).cast::<IdtEntry>();
    for index in 0..256 {
        entries.add(index).write(IdtEntry::interrupt(handler));
    }
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
