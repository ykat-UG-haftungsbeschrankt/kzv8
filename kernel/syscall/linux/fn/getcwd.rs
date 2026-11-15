/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use super::super::SyscallFrame;
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct GetCwd;

impl GetCwd {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::GetCwd;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let size = usize::try_from(frame.arguments[1]).map_err(|_| Error::Inval)?;
        let process = ThreadManager::current_process()?;
        let path = process.current_directory()?;
        let output = build_buffer(&path, size)?;
        let address = usize::try_from(frame.arguments[0]).map_err(|_| Error::Fault)?;
        if address == 0 {
            return Err(Error::Fault);
        }
        let mut address_space = process.address_space();
        address_space
            .write(address, &output)
            .map_err(|_| Error::Fault)?;
        i64::try_from(output.len()).map_err(|_| Error::Overflow)
    }
}

fn build_buffer(path: &[u8], size: usize) -> Result<Vec<u8>, Error> {
    if size == 0 {
        return Err(Error::Inval);
    }
    let required = path.len().checked_add(1).ok_or(Error::Overflow)?;
    if size < required {
        return Err(Error::Range);
    }
    let mut output = Vec::new();
    output.try_reserve_exact(required).map_err(|_| Error::NoMem)?;
    output.extend_from_slice(path);
    output.push(0);
    Ok(output)
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/getcwd.rs"]
mod tests;