/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::signal::SignalInfo;

pub(crate) struct Kill;

impl Kill {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Kill;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let process = ThreadManager::current_process()?;
        let target = frame.arguments[0] as i32;
        let signal = frame.arguments[1] as i32;
        process.send_signal(target, signal, SignalInfo::new(signal as u8, 0, 0, 0, 0, 0))?;
        Ok(0)
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/kill.rs"]
mod tests;