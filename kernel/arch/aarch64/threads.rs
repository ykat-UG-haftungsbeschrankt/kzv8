/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::mem::size_of;

#[cfg(feature = "aarch64-fpu-off")]
const FPU_CONTEXT_SIZE: usize = 16;
#[cfg(not(feature = "aarch64-fpu-off"))]
const FPU_CONTEXT_SIZE: usize = 528;

#[derive(Clone, Copy)]
#[repr(C, align(16))]
pub(crate) struct FpuContext {
    _image: [u8; FPU_CONTEXT_SIZE],
}

impl FpuContext {
    pub(crate) const INITIAL: Self = Self {
        _image: [0; FPU_CONTEXT_SIZE],
    };
}

const SAVE_FULL_FPU_CONTEXT: usize = if cfg!(feature = "aarch64-fpu-off") {
    0
} else {
    1
};

core::arch::global_asm!(
    ".section .text",
    ".global kz_aarch64_context_switch",
    ".type kz_aarch64_context_switch,%function",
    "kz_aarch64_context_switch:",
    ".if {save_full_fpu_context} == 1",
    "stp q0, q1, [x2, #0]",
    "stp q2, q3, [x2, #32]",
    "stp q4, q5, [x2, #64]",
    "stp q6, q7, [x2, #96]",
    "stp q8, q9, [x2, #128]",
    "stp q10, q11, [x2, #160]",
    "stp q12, q13, [x2, #192]",
    "stp q14, q15, [x2, #224]",
    "stp q16, q17, [x2, #256]",
    "stp q18, q19, [x2, #288]",
    "stp q20, q21, [x2, #320]",
    "stp q22, q23, [x2, #352]",
    "stp q24, q25, [x2, #384]",
    "stp q26, q27, [x2, #416]",
    "stp q28, q29, [x2, #448]",
    "stp q30, q31, [x2, #480]",
    "mrs x4, fpcr",
    "str x4, [x2, #512]",
    "mrs x4, fpsr",
    "str x4, [x2, #520]",
    ".endif",
    "stp x19, x20, [sp, #-16]!",
    "stp x21, x22, [sp, #-16]!",
    "stp x23, x24, [sp, #-16]!",
    "stp x25, x26, [sp, #-16]!",
    "stp x27, x28, [sp, #-16]!",
    "stp x29, x30, [sp, #-16]!",
    ".if {save_full_fpu_context} == 0",
    "stp d8, d9, [sp, #-16]!",
    "stp d10, d11, [sp, #-16]!",
    "stp d12, d13, [sp, #-16]!",
    "stp d14, d15, [sp, #-16]!",
    ".endif",
    "mov x4, sp",
    "str x4, [x0]",
    "mov sp, x1",
    ".if {save_full_fpu_context} == 1",
    "ldp q0, q1, [x3, #0]",
    "ldp q2, q3, [x3, #32]",
    "ldp q4, q5, [x3, #64]",
    "ldp q6, q7, [x3, #96]",
    "ldp q8, q9, [x3, #128]",
    "ldp q10, q11, [x3, #160]",
    "ldp q12, q13, [x3, #192]",
    "ldp q14, q15, [x3, #224]",
    "ldp q16, q17, [x3, #256]",
    "ldp q18, q19, [x3, #288]",
    "ldp q20, q21, [x3, #320]",
    "ldp q22, q23, [x3, #352]",
    "ldp q24, q25, [x3, #384]",
    "ldp q26, q27, [x3, #416]",
    "ldp q28, q29, [x3, #448]",
    "ldp q30, q31, [x3, #480]",
    "ldr x4, [x3, #512]",
    "msr fpcr, x4",
    "ldr x4, [x3, #520]",
    "msr fpsr, x4",
    ".else",
    "ldp d14, d15, [sp], #16",
    "ldp d12, d13, [sp], #16",
    "ldp d10, d11, [sp], #16",
    "ldp d8, d9, [sp], #16",
    ".endif",
    "ldp x29, x30, [sp], #16",
    "ldp x27, x28, [sp], #16",
    "ldp x25, x26, [sp], #16",
    "ldp x23, x24, [sp], #16",
    "ldp x21, x22, [sp], #16",
    "ldp x19, x20, [sp], #16",
    "ret",
    ".size kz_aarch64_context_switch, .-kz_aarch64_context_switch",
    save_full_fpu_context = const SAVE_FULL_FPU_CONTEXT,
);

unsafe extern "C" {
    fn kz_aarch64_context_switch(
        old_stack: *mut usize,
        new_stack: usize,
        old_fpu_context: *mut FpuContext,
        new_fpu_context: *const FpuContext,
    );
}

pub(crate) fn initialize_stack(stack: &mut [u8], entry: extern "C" fn() -> !) -> usize {
    #[cfg(feature = "aarch64-fpu-off")]
    const STACK_WORDS: usize = 20;
    #[cfg(not(feature = "aarch64-fpu-off"))]
    const STACK_WORDS: usize = 12;
    let stack_start = stack.as_mut_ptr() as usize;
    let top = (stack_start + stack.len()) & !0xf;
    let context_start = top - STACK_WORDS * size_of::<usize>();
    let context_offset = context_start - stack_start;
    let context_end = context_offset + STACK_WORDS * size_of::<usize>();
    let context = &mut stack[context_offset..context_end];
    context.fill(0);

    #[cfg(feature = "aarch64-fpu-off")]
    let entry_index = 9;
    #[cfg(not(feature = "aarch64-fpu-off"))]
    let entry_index = 1;
    let entry_offset = entry_index * size_of::<usize>();
    context[entry_offset..entry_offset + size_of::<usize>()]
        .copy_from_slice(&(entry as usize).to_ne_bytes());
    context_start
}

pub(crate) unsafe fn switch(
    old_stack: &mut usize,
    new_stack: usize,
    old_fpu_context: &mut FpuContext,
    new_fpu_context: &FpuContext,
) {
    unsafe { kz_aarch64_context_switch(old_stack, new_stack, old_fpu_context, new_fpu_context) }
}
