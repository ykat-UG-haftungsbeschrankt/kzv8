core::arch::global_asm!(
    ".section .text.vectors,\"ax\"",
    ".balign 2048",
    ".global aarch64_vector_table",
    "aarch64_vector_table:",
    ".rept 16",
    "b aarch64_unhandled_vector",
    ".balign 128",
    ".endr",
    "aarch64_unhandled_vector:",
    "msr daifset, #0xf",
    "1:",
    "wfi",
    "b 1b",
);

unsafe extern "C" {
    static aarch64_vector_table: u8;
}

pub(crate) unsafe fn initialize() {
    let vectors = core::ptr::addr_of!(aarch64_vector_table);
    core::arch::asm!(
        "msr vbar_el1, {vectors}",
        "isb",
        "msr daifset, #0xf",
        vectors = in(reg) vectors,
        options(nostack, preserves_flags),
    );
}

pub(crate) fn disable() -> u64 {
    let flags: u64;
    unsafe {
        core::arch::asm!(
            "mrs {flags}, daif",
            "msr daifset, #0xf",
            flags = out(reg) flags,
            options(nostack, preserves_flags),
        );
    }
    flags
}

pub(crate) fn enable() {
    unsafe {
        core::arch::asm!("msr daifclr, #0xf", options(nomem, nostack));
    }
}

pub(crate) fn restore(flags: u64) {
    unsafe {
        core::arch::asm!(
            "msr daif, {flags}",
            flags = in(reg) flags,
            options(nomem, nostack, preserves_flags),
        );
    }
}

pub(crate) fn enabled() -> bool {
    let flags: u64;
    unsafe {
        core::arch::asm!(
            "mrs {flags}, daif",
            flags = out(reg) flags,
            options(nomem, nostack, preserves_flags),
        );
    }
    flags & 0x3c0 == 0
}
