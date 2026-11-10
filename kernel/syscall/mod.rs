/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[path = "fn/mod.rs"]
mod r#fn;

use crate::kernel::error::Error;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SyscallFrame {
    pub(crate) number: u64,
    pub(crate) arguments: [u64; 6],
    pub(crate) result: i64,
}

pub(crate) fn dispatch(frame: &mut SyscallFrame) {
    let result = match frame.number {
        r#fn::exit::Exit::ID => r#fn::exit::Exit::handle(*frame),
        r#fn::exit_group::ExitGroup::ID => r#fn::exit_group::ExitGroup::handle(*frame),
        r#fn::getpid::GetPid::ID => r#fn::getpid::GetPid::handle(*frame),
        r#fn::gettid::GetTid::ID => r#fn::gettid::GetTid::handle(*frame),
        r#fn::sched_yield::SchedYield::ID => r#fn::sched_yield::SchedYield::handle(*frame),
        r#fn::write::Write::ID => r#fn::write::Write::handle(*frame),
        r#fn::clock_gettime::ClockGetTime::ID => {
            r#fn::clock_gettime::ClockGetTime::handle(*frame)
        }
        r#fn::mmap::Mmap::ID => r#fn::mmap::Mmap::handle(*frame),
        r#fn::munmap::Munmap::ID => r#fn::munmap::Munmap::handle(*frame),
        r#fn::mprotect::Mprotect::ID => r#fn::mprotect::Mprotect::handle(*frame),
        _ => Err(Error::NoSys),
    };
    frame.result = result.unwrap_or_else(|error| error as i64);
}