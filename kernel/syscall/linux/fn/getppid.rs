/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct GetPpid;

impl GetPpid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::GetPpid;

    pub(crate) fn handle(_frame: SyscallFrame) -> Result<i64, Error> {
        let process = ThreadManager::current_process()?;
        Ok(i64::from(process.parent_pid()))
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/getppid.rs"]
mod tests;