/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct Dup;

impl Dup {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Dup;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let source = u32::try_from(frame.arguments[0]).map_err(|_| Error::Badf)?;
        let process = ThreadManager::current_process()?;
        let (soft_limit, _) = process.resource_limit(5)?;
        let descriptor = Process::duplicate_file_descriptor_to_process(
            process.pid(),
            source,
            process.pid(),
        )?;
        if u64::from(descriptor) >= soft_limit {
            process.close_file_descriptor(descriptor)?;
            return Err(Error::Mfile);
        }
        Ok(i64::from(descriptor))
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/dup.rs"]
mod tests;