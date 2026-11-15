/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::syscall::{Syscall, SyscallNumbers};

impl SyscallNumbers for Syscall {
    const EXIT: u64 = 93;
    const EXIT_GROUP: u64 = 94;
    const GET_PID: u64 = 172;
    const GET_PPID: u64 = 173;
    const GET_TID: u64 = 178;
    const GET_UID: u64 = 174;
    const GET_EUID: u64 = 175;
    const GET_GID: u64 = 176;
    const GET_EGID: u64 = 177;
    const GET_CWD: u64 = 17;
    const GET_GROUPS: u64 = 158;
    const LSEEK: u64 = 62;
    const GETRESUID: u64 = 148;
    const GETRESGID: u64 = 150;
    const SETUID: u64 = 146;
    const SETGID: u64 = 144;
    const SETREUID: u64 = 145;
    const SETREGID: u64 = 143;
    const SETRESUID: u64 = 147;
    const SETRESGID: u64 = 149;
    const SET_GROUPS: u64 = 159;
    const UMASK: u64 = 166;
    const GETPGID: u64 = 155;
    const GETSID: u64 = 156;
    const SETPGID: u64 = 154;
    const SETSID: u64 = 157;
    const KILL: u64 = 129;
    const READLINKAT: u64 = 78;
    const UNLINKAT: u64 = 35;
    const GETRANDOM: u64 = 278;
    const NANOSLEEP: u64 = 101;
    const UNAME: u64 = 160;
    const GETRUSAGE: u64 = 165;
    const LINKAT: u64 = 37;
    const SYMLINKAT: u64 = 36;
    const DUP: u64 = 23;
    const GETRLIMIT: u64 = 163;
    const SETRLIMIT: u64 = 164;
    const GETPRIORITY: u64 = 141;
    const SETPRIORITY: u64 = 140;
    const CHDIR: u64 = 49;
    const SYNC: u64 = 81;
    const RT_SIGPROCMASK: u64 = 135;
    const RT_SIGPENDING: u64 = 136;
    const RT_SIGACTION: u64 = 134;
    const CHMOD: u64 = 53;
    const FCHMOD: u64 = 52;
    const CHOWN: u64 = 54;
    const FCHOWN: u64 = 55;
    const SCHED_YIELD: u64 = 124;
    const READ: u64 = 63;
    const WRITE: u64 = 64;
    const WRITEV: u64 = 66;
    const CLOSE: u64 = 57;
    const IOCTL: u64 = 29;
    const GETDENTS64: u64 = 61;
    const FSTAT: u64 = 80;
    const NEWFSTATAT: u64 = 79;
    const OPENAT: u64 = 56;
    const CLOCK_GET_TIME: u64 = 113;
    const MMAP: u64 = 222;
    const MUNMAP: u64 = 215;
    const MPROTECT: u64 = 226;
}