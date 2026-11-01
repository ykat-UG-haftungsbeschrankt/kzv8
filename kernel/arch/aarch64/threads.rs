use core::mem::size_of;

core::arch::global_asm!(
    ".section .text",
    ".global kz_aarch64_context_switch",
    ".type kz_aarch64_context_switch,%function",
    "kz_aarch64_context_switch:",
    "stp x19, x20, [sp, #-16]!",
    "stp x21, x22, [sp, #-16]!",
    "stp x23, x24, [sp, #-16]!",
    "stp x25, x26, [sp, #-16]!",
    "stp x27, x28, [sp, #-16]!",
    "stp x29, x30, [sp, #-16]!",
    "stp d8, d9, [sp, #-16]!",
    "stp d10, d11, [sp, #-16]!",
    "stp d12, d13, [sp, #-16]!",
    "stp d14, d15, [sp, #-16]!",
    "mov x2, sp",
    "str x2, [x0]",
    "mov sp, x1",
    "ldp d14, d15, [sp], #16",
    "ldp d12, d13, [sp], #16",
    "ldp d10, d11, [sp], #16",
    "ldp d8, d9, [sp], #16",
    "ldp x29, x30, [sp], #16",
    "ldp x27, x28, [sp], #16",
    "ldp x25, x26, [sp], #16",
    "ldp x23, x24, [sp], #16",
    "ldp x21, x22, [sp], #16",
    "ldp x19, x20, [sp], #16",
    "ret",
    ".size kz_aarch64_context_switch, .-kz_aarch64_context_switch",
);

unsafe extern "C" {
    fn kz_aarch64_context_switch(old_stack: *mut usize, new_stack: usize);
}

pub(crate) fn initialize_stack(stack: &mut [u8], entry: extern "C" fn() -> !) -> usize {
    const STACK_WORDS: usize = 20;
    let top = (stack.as_mut_ptr() as usize + stack.len()) & !0xf;
    let context = (top - STACK_WORDS * size_of::<usize>()) as *mut usize;
    unsafe {
        for index in 0..STACK_WORDS {
            context.add(index).write(0);
        }
        context.add(9).write(entry as usize);
    }
    context as usize
}

pub(crate) unsafe fn switch(old_stack: &mut usize, new_stack: usize) {
    unsafe { kz_aarch64_context_switch(old_stack, new_stack) }
}
