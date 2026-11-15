/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use super::path::{copy_user_path, resolve_at_directory};
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::vfs::VfsApi as _;

pub(crate) struct ReadlinkAt;

impl ReadlinkAt {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Readlinkat;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let process = ThreadManager::current_process()?;
        let mut address_space = process.address_space();
        let path = resolve_at_directory(
            copy_user_path(
                &address_space,
                usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?,
            )?,
            frame.arguments[0],
            &process,
        )?;
        let size = usize::try_from(frame.arguments[3]).map_err(|_| Error::Inval)?;
        let vfs = crate::kernel::vfs::mounted().ok_or(Error::NoDev)?;
        let target = vfs.read_link_blocking(&path, &process.credentials()?)?;
        let length = output_length(target.len(), size);
        if length != 0 {
            let output_address = usize::try_from(frame.arguments[2]).map_err(|_| Error::Fault)?;
            if output_address == 0 {
                return Err(Error::Fault);
            }
            address_space
                .write(output_address, &target.as_bytes()[..length])
                .map_err(|_| Error::Fault)?;
        }
        i64::try_from(length).map_err(|_| Error::Overflow)
    }
}

fn output_length(target_length: usize, buffer_size: usize) -> usize {
    target_length.min(buffer_size)
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/readlinkat.rs"]
mod tests;