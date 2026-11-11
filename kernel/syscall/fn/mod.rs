/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) mod clock_gettime;
pub(crate) mod close;
pub(crate) mod exit;
pub(crate) mod exit_group;
pub(crate) mod getpid;
pub(crate) mod gettid;
pub(crate) mod mmap;
pub(crate) mod mprotect;
pub(crate) mod munmap;
pub(crate) mod openat;
pub(crate) mod read;
pub(crate) mod sched_yield;
pub(crate) mod write;

const PAGE_SIZE: usize = crate::kernel::memory::r#virtual::PAGE_SIZE;
const PROT_READ: u64 = 1;
const PROT_WRITE: u64 = 2;
const PROT_EXEC: u64 = 4;

fn protection_to_vm(protection: u64) -> Option<i32> {
    if protection & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0
        || protection & (PROT_WRITE | PROT_EXEC) == (PROT_WRITE | PROT_EXEC)
    {
        return None;
    }
    Some(if protection & PROT_EXEC != 0 {
        4
    } else if protection & PROT_WRITE != 0 {
        2
    } else if protection & PROT_READ != 0 {
        1
    } else {
        0
    })
}