/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct GetPgid;
pub(crate) struct GetSid;
pub(crate) struct SetPgid;
pub(crate) struct SetSid;

fn pid_argument(value: u64) -> i32 {
    value as i32
}

impl GetPgid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Getpgid;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let process = ThreadManager::current_process()?;
        Ok(i64::from(process.process_group_for(pid_argument(frame.arguments[0]))?))
    }
}

impl GetSid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Getsid;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let process = ThreadManager::current_process()?;
        Ok(i64::from(process.session_for(pid_argument(frame.arguments[0]))?))
    }
}

impl SetPgid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Setpgid;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let process = ThreadManager::current_process()?;
        process.set_process_group(
            pid_argument(frame.arguments[0]),
            pid_argument(frame.arguments[1]),
        )?;
        Ok(0)
    }
}

impl SetSid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Setsid;

    pub(crate) fn handle(_frame: SyscallFrame) -> Result<i64, Error> {
        let process = ThreadManager::current_process()?;
        Ok(i64::from(process.create_session()?))
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/process_group.rs"]
mod tests;