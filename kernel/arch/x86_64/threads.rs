/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::mem::size_of;
use core::ptr::{addr_of, copy_nonoverlapping, write_bytes};

use crate::kernel::scheduler::sync::IrqSpinLock;

#[cfg(feature = "fpu-x87")]
const FPU_CONTEXT_SIZE: usize = 108;
#[cfg(all(feature = "fpu-sse", not(feature = "fpu-avx")))]
const FPU_CONTEXT_SIZE: usize = 512;
#[cfg(not(any(feature = "fpu-x87", feature = "fpu-sse", feature = "fpu-avx")))]
const FPU_CONTEXT_SIZE: usize = 512;
#[cfg(feature = "fpu-avx")]
const FPU_CONTEXT_SIZE: usize = 832;

#[derive(Clone, Copy)]
#[repr(C, align(64))]
pub(crate) struct FpuContext {
    _image: [u8; FPU_CONTEXT_SIZE],
}

impl FpuContext {
    pub(crate) const INITIAL: Self = {
        let mut image = [0; FPU_CONTEXT_SIZE];
        if cfg!(feature = "fpu-x87") {
            image[0] = 0x7f;
            image[1] = 0x03;
            image[8] = 0xff;
            image[9] = 0xff;
        } else if !cfg!(feature = "fpu-avx") {
            image[0] = 0x7f;
            image[1] = 0x03;
            image[24] = 0x80;
            image[25] = 0x1f;
        }
        Self { _image: image }
    };
}

const TLS_BLOCK_SIZE: usize = 4096;

//@FIXME: INDEXTYPE(TLS_BLOCKS); CPU-ID-indexed TLS block table; Vec access is O(1), keep Vec.
static TLS_BLOCKS: IrqSpinLock<Vec<Option<alloc::boxed::Box<[u8]>>>> =
    IrqSpinLock::new(Vec::new());

unsafe extern "C" {
    static __tls_start: u8;
    static __tls_data_end: u8;
    static __tls_end: u8;
}

macro_rules! define_context_switch {
    ([$($save:literal),+], [$($restore:literal),+]) => {
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
            $($save,)+
            "mov [rdi], rsp",
            "mov rsp, rsi",
            $($restore,)+
            "pop r15",
            "pop r14",
            "pop r13",
            "pop r12",
            "pop rbp",
            "pop rbx",
            "ret",
            ".size kz_x86_64_context_switch, .-kz_x86_64_context_switch",
        );
    };
}

#[cfg(feature = "fpu-x87")]
define_context_switch!(["fnsave [rdx]"], ["frstor [rcx]"]);

#[cfg(all(feature = "fpu-sse", not(feature = "fpu-avx")))]
define_context_switch!(["fxsave64 [rdx]"], ["fxrstor64 [rcx]"]);

#[cfg(not(any(feature = "fpu-x87", feature = "fpu-sse", feature = "fpu-avx")))]
define_context_switch!(["fxsave64 [rdx]"], ["fxrstor64 [rcx]"]);

#[cfg(feature = "fpu-avx")]
define_context_switch!(
    ["mov r8, rdx", "mov eax, 7", "xor edx, edx", "xsave64 [r8]"],
    ["mov r8, rcx", "mov eax, 7", "xor edx, edx", "xrstor64 [r8]"]
);

unsafe extern "C" {
    fn kz_x86_64_context_switch(
        old_stack: *mut usize,
        new_stack: usize,
        old_fpu_context: *mut FpuContext,
        new_fpu_context: *const FpuContext,
    );
}

pub(crate) fn initialize_thread_local_storage(cpu_id: usize) -> bool {
    let Some(tls_block) = tls_block(cpu_id) else {
        return false;
    };

    unsafe {
        let tls_start = addr_of!(__tls_start) as usize;
        let tls_data_end = addr_of!(__tls_data_end) as usize;
        let tls_end = addr_of!(__tls_end) as usize;
        let Some((tls_size, tls_data_size)) = crate::kernel::lib::tls::template_sizes(
            tls_start,
            tls_data_end,
            tls_end,
            TLS_BLOCK_SIZE - 2 * size_of::<usize>(),
        ) else {
            return false;
        };

        write_bytes(tls_block, 0, TLS_BLOCK_SIZE);
        copy_nonoverlapping(tls_start as *const u8, tls_block, tls_data_size);

        let thread_pointer = tls_block.add(tls_size) as usize;
        (thread_pointer as *mut usize).write(thread_pointer);
        ((thread_pointer + size_of::<usize>()) as *mut usize).write(0);

        let thread_pointer = thread_pointer as u64;
        core::arch::asm!(
            "wrmsr",
            in("ecx") 0xc000_0100u32,
            in("eax") thread_pointer as u32,
            in("edx") (thread_pointer >> 32) as u32,
            options(nostack, preserves_flags),
        );
    }
    true
}

fn tls_block(cpu_id: usize) -> Option<*mut u8> {
    let required = cpu_id.checked_add(1)?;
    let mut blocks = TLS_BLOCKS.lock();
    blocks.resize_with(required, || None);
    if blocks[cpu_id].is_none() {
        let mut block = Vec::with_capacity(TLS_BLOCK_SIZE);
        block.resize(TLS_BLOCK_SIZE, 0);
        blocks[cpu_id] = Some(block.into_boxed_slice());
    }
    blocks[cpu_id].as_mut().map(|block| block.as_mut_ptr())
}

pub(crate) fn initialize_stack(stack: &mut [u8], entry: extern "C" fn() -> !) -> usize {
    const STACK_WORDS: usize = 8;
    let stack_start = stack.as_mut_ptr() as usize;
    let top = (stack_start + stack.len()) & !0xf;
    let context_start = top - STACK_WORDS * size_of::<usize>();
    let context_offset = context_start - stack_start;
    let context_end = context_offset + STACK_WORDS * size_of::<usize>();
    let context = &mut stack[context_offset..context_end];
    context.fill(0);

    let entry_offset = 6 * size_of::<usize>();
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
    unsafe { kz_x86_64_context_switch(old_stack, new_stack, old_fpu_context, new_fpu_context) }
}
