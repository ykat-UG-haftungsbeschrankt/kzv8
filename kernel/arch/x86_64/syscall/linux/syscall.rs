/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::syscall::SyscallFrame;
use crate::kernel::arch::traits::CpuArchitecture as _;
use crate::kernel::arch::syscall::{Syscall, SyscallNumbers};

const FPU_MODE: usize = if cfg!(feature = "fpu-x87") {
    0
} else if cfg!(feature = "fpu-avx") {
    2
} else {
    1
};

const IA32_EFER: u32 = 0xc000_0080;
const IA32_STAR: u32 = 0xc000_0081;
const IA32_LSTAR: u32 = 0xc000_0082;
const IA32_FMASK: u32 = 0xc000_0084;
const IA32_FS_BASE: u32 = 0xc000_0100;
const EFER_SCE: u64 = 1;
const RFLAGS_CARRY: u64 = 1;
const STAR_VALUE: u64 = (0x23u64 << 48) | (0x08u64 << 32);
const RFLAGS_SYSCALL_MASK: u64 = (1 << 8) | (1 << 10) | (1 << 16);

core::arch::global_asm!(
    ".section .text,\"ax\"",
    ".global x86_syscall_entry",
    ".type x86_syscall_entry,@function",
    "x86_syscall_entry:",
    "swapgs",
    "cli",
    "mov qword ptr gs:[8], rsp",
    "mov rsp, qword ptr gs:[0]",
    "push qword ptr gs:[8]",
    "push r11",
    "push rcx",
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
    "mov r13, rsp",
    "fnsave [rsp]",
    ".elseif {fpu_mode} == 1",
    "sub rsp, 528",
    "and rsp, -16",
    "mov r13, rsp",
    "fxsave64 [rsp]",
    ".else",
    "sub rsp, 896",
    "and rsp, -64",
    "mov r13, rsp",
    "mov eax, 7",
    "xor edx, edx",
    "xsave64 [rsp]",
    ".endif",
    "mov rdi, r12",
    "call x86_syscall_dispatch",
    ".if {fpu_mode} == 0",
    "frstor [r13]",
    ".elseif {fpu_mode} == 1",
    "fxrstor64 [r13]",
    ".else",
    "mov eax, 7",
    "xor edx, edx",
    "xrstor64 [r13]",
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
    "push 0x2b",
    "push qword ptr [rsp + 24]",
    "push qword ptr [rsp + 24]",
    "push 0x33",
    "push qword ptr [rsp + 32]",
    "swapgs",
    "iretq",
    ".size x86_syscall_entry, .-x86_syscall_entry",
    fpu_mode = const FPU_MODE,
);

#[repr(C)]
struct SavedRegisters {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rdi: u64,
    rsi: u64,
    rbp: u64,
    rdx: u64,
    rcx: u64,
    rbx: u64,
    rax: u64,
    // The interrupt-return frame follows the saved general-purpose registers.
    user_return_frame: [u64; 3],
}

#[no_mangle]
extern "C" fn x86_syscall_dispatch(registers: *mut SavedRegisters) {
    let registers = unsafe { &mut *registers };
    let mut frame = SyscallFrame {
        number: registers.rax,
        arguments: [
            registers.rdi,
            registers.rsi,
            registers.rdx,
            registers.r10,
            registers.r8,
            registers.r9,
        ],
        windows_arguments: [registers.r10, registers.rdx, registers.r8, registers.r9],
        result: 0,
        errno: 0,
    };
    crate::kernel::syscall::dispatch(&mut frame);
    if crate::kernel::syscall::darwin::has_syscall_class(frame.number) {
        let user_flags = &mut registers.user_return_frame[1];
        if frame.errno != 0 {
            *user_flags |= RFLAGS_CARRY;
        } else {
            *user_flags &= !RFLAGS_CARRY;
        }
    }
    registers.rax = frame.result as u64;
}

pub(crate) fn initialize() -> bool {
    let Some(_segments) = super::segments::initialize() else {
        return false;
    };
    let entry = x86_syscall_entry as *const () as usize as u64;
    let mut efer = read_msr(IA32_EFER);
    efer |= EFER_SCE;
    write_msr(IA32_STAR, STAR_VALUE);
    write_msr(IA32_LSTAR, entry);
    write_msr(IA32_FMASK, RFLAGS_SYSCALL_MASK);
    write_msr(IA32_EFER, efer);
    true
}

pub(crate) unsafe fn enter_user(entry: usize, stack_pointer: usize) -> ! {
    if !initialize() || stack_pointer % 16 != 0 {
        crate::kernel::arch::CpuArchitecture::halt();
    }
    let kernel_stack_pointer: usize;
    unsafe {
        core::arch::asm!(
            "mov {stack}, rsp",
            stack = out(reg) kernel_stack_pointer,
            options(nomem, nostack, preserves_flags),
        );
    }
    if !super::segments::set_kernel_stack(kernel_stack_pointer) {
        crate::kernel::arch::CpuArchitecture::halt();
    }
    unsafe {
        core::arch::asm!(
            "mov ax, 0x2b",
            "mov ds, ax",
            "mov es, ax",
            "push 0x2b",
            "push rsi",
            "push 0x202",
            "push 0x33",
            "push rdi",
            "swapgs",
            "iretq",
            in("rdi") entry,
            in("rsi") stack_pointer,
            options(noreturn),
        );
    }
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
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") index,
            in("eax") value as u32,
            in("edx") (value >> 32) as u32,
            options(nostack, preserves_flags),
        );
    }
}

pub(crate) fn set_user_fs_base(value: u64) {
    write_msr(IA32_FS_BASE, value);
}

pub(crate) fn user_fs_base() -> u64 {
    read_msr(IA32_FS_BASE)
}

unsafe extern "C" {
    fn x86_syscall_entry();
}

impl SyscallNumbers for Syscall {
    const EXIT: u64 = 60;
    const EXIT_GROUP: u64 = 231;
    const GET_PID: u64 = 39;
    const GET_PPID: u64 = 110;
    const GET_TID: u64 = 186;
    const GET_UID: u64 = 102;
    const GET_EUID: u64 = 107;
    const GET_GID: u64 = 104;
    const GET_EGID: u64 = 108;
    const GET_CWD: u64 = 79;
    const GET_GROUPS: u64 = 115;
    const LSEEK: u64 = 8;
    const GETRESUID: u64 = 118;
    const GETRESGID: u64 = 120;
    const SETUID: u64 = 105;
    const SETGID: u64 = 106;
    const SETREUID: u64 = 113;
    const SETREGID: u64 = 114;
    const SETRESUID: u64 = 117;
    const SETRESGID: u64 = 119;
    const SET_GROUPS: u64 = 116;
    const UMASK: u64 = 95;
    const GETPGID: u64 = 121;
    const GETSID: u64 = 124;
    const SETPGID: u64 = 109;
    const SETSID: u64 = 112;
    const KILL: u64 = 62;
    const READLINKAT: u64 = 267;
    const UNLINKAT: u64 = 263;
    const GETRANDOM: u64 = 318;
    const NANOSLEEP: u64 = 35;
    const UNAME: u64 = 63;
    const GETRUSAGE: u64 = 98;
    const LINKAT: u64 = 265;
    const SYMLINKAT: u64 = 266;
    const DUP: u64 = 32;
    const GETRLIMIT: u64 = 97;
    const SETRLIMIT: u64 = 160;
    const GETPRIORITY: u64 = 140;
    const SETPRIORITY: u64 = 141;
    const CHDIR: u64 = 80;
    const SYNC: u64 = 162;
    const RT_SIGPROCMASK: u64 = 14;
    const RT_SIGPENDING: u64 = 127;
    const RT_SIGACTION: u64 = 13;
    const CHMOD: u64 = 90;
    const FCHMOD: u64 = 91;
    const CHOWN: u64 = 92;
    const FCHOWN: u64 = 93;
    const SCHED_YIELD: u64 = 24;
    const READ: u64 = 0;
    const WRITE: u64 = 1;
    const WRITEV: u64 = 20;
    const CLOSE: u64 = 3;
    const IOCTL: u64 = 16;
    const GETDENTS64: u64 = 217;
    const FSTAT: u64 = 5;
    const NEWFSTATAT: u64 = 262;
    const OPENAT: u64 = 257;
    const CLOCK_GET_TIME: u64 = 228;
    const MMAP: u64 = 9;
    const MUNMAP: u64 = 11;
    const MPROTECT: u64 = 10;
}