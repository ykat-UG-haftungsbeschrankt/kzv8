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

const AT_SYMLINK_FOLLOW: u32 = 0x400;
const AT_EMPTY_PATH: u32 = 0x1000;
const LINKAT_KNOWN_FLAGS: u32 = AT_SYMLINK_FOLLOW | AT_EMPTY_PATH;

pub(crate) struct LinkAt;
pub(crate) struct SymlinkAt;

impl LinkAt {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Linkat;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        validate_linkat_flags(frame.arguments[4] as u32)?;
        let process = ThreadManager::current_process()?;
        let address_space = process.address_space();
        let source = resolve_at_directory(
            copy_user_path(
                &address_space,
                usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?,
            )?,
            frame.arguments[0],
            &process,
        )?;
        let destination = resolve_at_directory(
            copy_user_path(
                &address_space,
                usize::try_from(frame.arguments[3]).map_err(|_| Error::Fault)?,
            )?,
            frame.arguments[2],
            &process,
        )?;
        let vfs = crate::kernel::vfs::mounted().ok_or(Error::NoDev)?;
        vfs.hard_link_blocking(&source, &destination, &process.credentials()?)?;
        Ok(0)
    }
}

impl SymlinkAt {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Symlinkat;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let process = ThreadManager::current_process()?;
        let address_space = process.address_space();
        let target = copy_user_path(
            &address_space,
            usize::try_from(frame.arguments[0]).map_err(|_| Error::Fault)?,
        )?;
        let link_path = resolve_at_directory(
            copy_user_path(
                &address_space,
                usize::try_from(frame.arguments[2]).map_err(|_| Error::Fault)?,
            )?,
            frame.arguments[1],
            &process,
        )?;
        let vfs = crate::kernel::vfs::mounted().ok_or(Error::NoDev)?;
        vfs.symlink_blocking(&target, &link_path, &process.credentials()?)?;
        Ok(0)
    }
}

fn validate_linkat_flags(flags: u32) -> Result<(), Error> {
    if flags & !LINKAT_KNOWN_FLAGS != 0 {
        Err(Error::Inval)
    } else if flags != 0 {
        Err(Error::NoSys)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/links.rs"]
mod tests;