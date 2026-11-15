/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) mod clock_gettime;
#[cfg(target_arch = "x86_64")]
pub(crate) mod arch_prctl;
pub(crate) mod chdir;
pub(crate) mod close;
pub(crate) mod credentials;
pub(crate) mod dup;
pub(crate) mod getcwd;
pub(crate) mod metadata;
pub(crate) mod exit;
pub(crate) mod getgroups;
pub(crate) mod getrandom;
pub(crate) mod getrusage;
pub(crate) mod rlimit;
pub(crate) mod signal_mask;
pub(crate) mod signal_pending;
pub(crate) mod signal_action;
pub(crate) mod stat;
pub(crate) mod exit_group;
pub(crate) mod getdents64;
pub(crate) mod getpid;
pub(crate) mod process_group;
pub(crate) mod priority;
pub(crate) mod getppid;
pub(crate) mod gettid;
pub(crate) mod kill;
pub(crate) mod ioctl;
pub(crate) mod mmap;
pub(crate) mod mprotect;
pub(crate) mod munmap;
pub(crate) mod lseek;
pub(crate) mod links;
pub(crate) mod nanosleep;
pub(crate) mod openat;
pub(crate) mod path;
pub(crate) mod read;
pub(crate) mod readlinkat;
pub(crate) mod sched_yield;
pub(crate) mod sync;
pub(crate) mod unsupported;
pub(crate) mod umask;
pub(crate) mod uname;
pub(crate) mod unlinkat;
pub(crate) mod write;
pub(crate) mod writev;

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