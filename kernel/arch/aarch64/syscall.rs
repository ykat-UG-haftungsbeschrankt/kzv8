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
    const GET_TID: u64 = 178;
    const SCHED_YIELD: u64 = 124;
    const WRITE: u64 = 64;
    const CLOCK_GET_TIME: u64 = 113;
    const MMAP: u64 = 222;
    const MUNMAP: u64 = 215;
    const MPROTECT: u64 = 226;
}