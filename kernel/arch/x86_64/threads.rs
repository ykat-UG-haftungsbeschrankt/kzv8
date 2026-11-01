use core::mem::size_of;

core::arch::global_asm!(
    ".section .text",
    ".global kz_x86_64_context_switch",
    ".type kz_x86_64_context_switch,@function",
    "kz_x86_64_context_switch:",
    "push rbx",
    "push rbp",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov [rdi], rsp",
    "mov rsp, rsi",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbp",
    "pop rbx",
    "ret",
    ".size kz_x86_64_context_switch, .-kz_x86_64_context_switch",
);

unsafe extern "C" {
    fn kz_x86_64_context_switch(old_stack: *mut usize, new_stack: usize);
}

pub(crate) fn initialize_stack(stack: &mut [u8], entry: extern "C" fn() -> !) -> usize {
    const STACK_WORDS: usize = 8;
    let top = (stack.as_mut_ptr() as usize + stack.len()) & !0xf;
    let context = (top - STACK_WORDS * size_of::<usize>()) as *mut usize;
    unsafe {
        context.add(0).write(0);
        context.add(1).write(0);
        context.add(2).write(0);
        context.add(3).write(0);
        context.add(4).write(0);
        context.add(5).write(0);
        context.add(6).write(entry as usize);
        context.add(7).write(0);
    }
    context as usize
}

pub(crate) unsafe fn switch(old_stack: &mut usize, new_stack: usize) {
    unsafe { kz_x86_64_context_switch(old_stack, new_stack) }
}
