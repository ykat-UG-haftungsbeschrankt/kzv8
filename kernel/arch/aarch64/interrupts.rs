/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

core::arch::global_asm!(
    ".section .text.vectors,\"ax\"",
    ".balign 2048",
    ".global aarch64_vector_table",
    "aarch64_vector_table:",
    ".rept 4",
    "b aarch64_unhandled_vector",
    ".balign 128",
    ".endr",
    "b aarch64_page_fault_entry",
    ".balign 128",
    "b aarch64_irq_entry",
    ".balign 128",
    ".rept 10",
    "b aarch64_unhandled_vector",
    ".balign 128",
    ".endr",
    "aarch64_unhandled_vector:",
    "msr daifset, #0xf",
    "1:",
    "wfi",
    "b 1b",
    "aarch64_irq_entry:",
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
    "stp q0, q1, [sp, #256]",
    "stp q2, q3, [sp, #288]",
    "stp q4, q5, [sp, #320]",
    "stp q6, q7, [sp, #352]",
    "stp q8, q9, [sp, #384]",
    "stp q10, q11, [sp, #416]",
    "stp q12, q13, [sp, #448]",
    "stp q14, q15, [sp, #480]",
    "stp q16, q17, [sp, #512]",
    "stp q18, q19, [sp, #544]",
    "stp q20, q21, [sp, #576]",
    "stp q22, q23, [sp, #608]",
    "stp q24, q25, [sp, #640]",
    "stp q26, q27, [sp, #672]",
    "stp q28, q29, [sp, #704]",
    "stp q30, q31, [sp, #736]",
    "mrs x0, fpcr",
    "str x0, [sp, #768]",
    "mrs x0, fpsr",
    "str x0, [sp, #776]",
    "mrs x0, elr_el1",
    "str x0, [sp, #784]",
    "mrs x0, spsr_el1",
    "str x0, [sp, #792]",
    "bl aarch64_irq_dispatch",
    "ldr x0, [sp, #768]",
    "msr fpcr, x0",
    "ldr x0, [sp, #776]",
    "msr fpsr, x0",
    "ldr x0, [sp, #784]",
    "msr elr_el1, x0",
    "ldr x0, [sp, #792]",
    "msr spsr_el1, x0",
    "ldp q30, q31, [sp, #736]",
    "ldp q28, q29, [sp, #704]",
    "ldp q26, q27, [sp, #672]",
    "ldp q24, q25, [sp, #640]",
    "ldp q22, q23, [sp, #608]",
    "ldp q20, q21, [sp, #576]",
    "ldp q18, q19, [sp, #544]",
    "ldp q16, q17, [sp, #512]",
    "ldp q14, q15, [sp, #480]",
    "ldp q12, q13, [sp, #448]",
    "ldp q10, q11, [sp, #416]",
    "ldp q8, q9, [sp, #384]",
    "ldp q6, q7, [sp, #352]",
    "ldp q4, q5, [sp, #320]",
    "ldp q2, q3, [sp, #288]",
    "ldp q0, q1, [sp, #256]",
    "ldr x30, [sp, #240]",
    "ldp x28, x29, [sp, #224]",
    "ldp x26, x27, [sp, #208]",
    "ldp x24, x25, [sp, #192]",
    "ldp x22, x23, [sp, #176]",
    "ldp x20, x21, [sp, #160]",
    "ldp x18, x19, [sp, #144]",
    "ldp x16, x17, [sp, #128]",
    "ldp x14, x15, [sp, #112]",
    "ldp x12, x13, [sp, #96]",
    "ldp x10, x11, [sp, #80]",
    "ldp x8, x9, [sp, #64]",
    "ldp x6, x7, [sp, #48]",
    "ldp x4, x5, [sp, #32]",
    "ldp x2, x3, [sp, #16]",
    "ldp x0, x1, [sp, #0]",
    "add sp, sp, #816",
    "eret",
    "aarch64_page_fault_entry:",
    "sub sp, sp, #800",
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
    "stp q0, q1, [sp, #256]",
    "stp q2, q3, [sp, #288]",
    "stp q4, q5, [sp, #320]",
    "stp q6, q7, [sp, #352]",
    "stp q8, q9, [sp, #384]",
    "stp q10, q11, [sp, #416]",
    "stp q12, q13, [sp, #448]",
    "stp q14, q15, [sp, #480]",
    "stp q16, q17, [sp, #512]",
    "stp q18, q19, [sp, #544]",
    "stp q20, q21, [sp, #576]",
    "stp q22, q23, [sp, #608]",
    "stp q24, q25, [sp, #640]",
    "stp q26, q27, [sp, #672]",
    "stp q28, q29, [sp, #704]",
    "stp q30, q31, [sp, #736]",
    "mrs x0, fpcr",
    "str x0, [sp, #768]",
    "mrs x0, fpsr",
    "str x0, [sp, #776]",
    "bl aarch64_page_fault_dispatch",
    "cbz x0, aarch64_unhandled_vector",
    "ldr x0, [sp, #768]",
    "msr fpcr, x0",
    "ldr x0, [sp, #776]",
    "msr fpsr, x0",
    "ldp q30, q31, [sp, #736]",
    "ldp q28, q29, [sp, #704]",
    "ldp q26, q27, [sp, #672]",
    "ldp q24, q25, [sp, #640]",
    "ldp q22, q23, [sp, #608]",
    "ldp q20, q21, [sp, #576]",
    "ldp q18, q19, [sp, #544]",
    "ldp q16, q17, [sp, #512]",
    "ldp q14, q15, [sp, #480]",
    "ldp q12, q13, [sp, #448]",
    "ldp q10, q11, [sp, #416]",
    "ldp q8, q9, [sp, #384]",
    "ldp q6, q7, [sp, #352]",
    "ldp q4, q5, [sp, #320]",
    "ldp q2, q3, [sp, #288]",
    "ldp q0, q1, [sp, #256]",
    "ldr x30, [sp, #240]",
    "ldp x28, x29, [sp, #224]",
    "ldp x26, x27, [sp, #208]",
    "ldp x24, x25, [sp, #192]",
    "ldp x22, x23, [sp, #176]",
    "ldp x20, x21, [sp, #160]",
    "ldp x18, x19, [sp, #144]",
    "ldp x16, x17, [sp, #128]",
    "ldp x14, x15, [sp, #112]",
    "ldp x12, x13, [sp, #96]",
    "ldp x10, x11, [sp, #80]",
    "ldp x8, x9, [sp, #64]",
    "ldp x6, x7, [sp, #48]",
    "ldp x4, x5, [sp, #32]",
    "ldp x2, x3, [sp, #16]",
    "ldp x0, x1, [sp, #0]",
    "add sp, sp, #800",
    "eret",
);

unsafe extern "C" {
    static aarch64_vector_table: u8;
}

const GICD_BASE: usize = 0x0800_0000;
const GICC_BASE: usize = 0x0801_0000;
const GIC_SPI_FIRST: usize = 32;
const GIC_SPI_END: usize = 64;

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

pub(crate) fn initialize_controller() -> bool {
    let interrupt_count =
        unsafe { ((core::ptr::read_volatile((GICD_BASE + 4) as *const u32) & 0x1f) + 1) * 32 }
            as usize;
    if interrupt_count < GIC_SPI_END {
        return false;
    }

    unsafe {
        core::ptr::write_volatile(GICD_BASE as *mut u32, 0);
        core::ptr::write_volatile((GICD_BASE + 0x184) as *mut u32, u32::MAX);
        core::ptr::write_volatile((GICD_BASE + 0x284) as *mut u32, u32::MAX);
        core::ptr::write_volatile((GICD_BASE + 0x84) as *mut u32, u32::MAX);
        core::ptr::write_volatile((GICD_BASE + 0xc08) as *mut u32, 0);
        core::ptr::write_volatile((GICD_BASE + 0xc0c) as *mut u32, 0);
        let timer_group = (GICD_BASE + 0x80) as *mut u32;
        let group = core::ptr::read_volatile(timer_group);
        core::ptr::write_volatile(
            timer_group,
            group | (1 << crate::kernel::arch::timer::INTERRUPT_ID),
        );
        core::ptr::write_volatile(
            (GICD_BASE + 0x400 + crate::kernel::arch::timer::INTERRUPT_ID as usize) as *mut u8,
            0x80,
        );
        core::ptr::write_volatile(
            (GICD_BASE + 0x100) as *mut u32,
            1 << crate::kernel::arch::timer::INTERRUPT_ID,
        );
        for interrupt in GIC_SPI_FIRST..GIC_SPI_END {
            core::ptr::write_volatile((GICD_BASE + 0x400 + interrupt) as *mut u8, 0x80);
            core::ptr::write_volatile((GICD_BASE + 0x800 + interrupt) as *mut u8, 1);
        }
        core::ptr::write_volatile((GICD_BASE + 0x104) as *mut u32, u32::MAX);
        core::ptr::write_volatile(GICD_BASE as *mut u32, 2);
        core::arch::asm!("dsb sy", "isb", options(nostack, preserves_flags));
    }
    initialize_cpu_interface()
}

pub(crate) fn initialize_secondary_controller() -> bool {
    initialize_cpu_interface()
}

fn initialize_cpu_interface() -> bool {
    unsafe {
        core::ptr::write_volatile((GICC_BASE + 0x04) as *mut u32, 0xff);
        core::ptr::write_volatile((GICC_BASE + 0x08) as *mut u32, 0);
        core::ptr::write_volatile(GICC_BASE as *mut u32, 2);
        core::arch::asm!("dsb sy", "isb", options(nostack, preserves_flags));
    }
    true
}

#[no_mangle]
pub extern "C" fn aarch64_irq_dispatch() {
    let acknowledge = unsafe { core::ptr::read_volatile((GICC_BASE + 0x0c) as *const u32) };
    let interrupt_id = acknowledge & 0x3ff;
    if interrupt_id >= 1020 {
        return;
    }
    if interrupt_id == crate::kernel::arch::timer::INTERRUPT_ID {
        crate::kernel::arch::timer::handle_interrupt();
        unsafe {
            core::ptr::write_volatile((GICC_BASE + 0x10) as *mut u32, acknowledge);
            core::arch::asm!("dsb sy", options(nostack, preserves_flags));
        }
        crate::kernel::scheduler::thread::ThreadManager::timer_tick();
        return;
    }
    crate::kz_handle_virtio_interrupt();
    unsafe {
        core::ptr::write_volatile((GICC_BASE + 0x10) as *mut u32, acknowledge);
        core::arch::asm!("dsb sy", options(nostack, preserves_flags));
    }
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

#[no_mangle]
extern "C" fn aarch64_page_fault_dispatch() -> u64 {
    let syndrome: u64;
    let address: u64;
    unsafe {
        core::arch::asm!(
            "mrs {syndrome}, esr_el1",
            "mrs {address}, far_el1",
            syndrome = out(reg) syndrome,
            address = out(reg) address,
            options(nomem, nostack, preserves_flags),
        );
    }

    let exception_class = (syndrome >> 26) & 0x3f;
    let fault_status = syndrome & 0x3f;
    if !(0x4..=0x7).contains(&fault_status) {
        return 0;
    }
    let access = match exception_class {
        0x21 => crate::kernel::memory::r#virtual::FaultAccess::Execute,
        0x25 if syndrome & (1 << 6) != 0 => {
            crate::kernel::memory::r#virtual::FaultAccess::Write
        }
        0x25 => crate::kernel::memory::r#virtual::FaultAccess::Read,
        _ => return 0,
    };
    crate::kernel::memory::r#virtual::resolve_page_fault(address as usize, access) as u64
}
