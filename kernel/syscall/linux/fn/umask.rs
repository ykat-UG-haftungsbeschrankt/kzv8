/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct Umask;

impl Umask {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Umask;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let previous = ThreadManager::current_process()?.set_umask(frame.arguments[0] as u32);
        Ok(i64::from(previous))
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/umask.rs"]
mod tests;