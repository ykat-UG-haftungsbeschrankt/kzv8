/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use super::path::{copy_user_path, resolve_at_directory};
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::vfs::VfsApi as _;

const AT_REMOVEDIR: u32 = 0x200;

pub(crate) struct UnlinkAt;

impl UnlinkAt {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Unlinkat;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let flags = frame.arguments[3] as u32;
        validate_flags(flags)?;
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
        let vfs = crate::kernel::vfs::mounted().ok_or(Error::NoDev)?;
        vfs.unlink_blocking(&path, &process.credentials()?)?;
        Ok(0)
    }
}

fn validate_flags(flags: u32) -> Result<(), Error> {
    match flags {
        0 => Ok(()),
        AT_REMOVEDIR => Err(Error::NoSys),
        _ => Err(Error::Inval),
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/unlinkat.rs"]
mod tests;