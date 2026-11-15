/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use super::path::{copy_user_path, resolve_at_current_directory};
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::vfs::{Node, VfsApi as _};

pub(crate) struct Chdir;

impl Chdir {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Chdir;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let process = ThreadManager::current_process()?;
        let address_space = process.address_space();
        let path = resolve_at_current_directory(
            copy_user_path(
                &address_space,
                usize::try_from(frame.arguments[0]).map_err(|_| Error::Fault)?,
            )?,
            &process.current_directory()?,
        )?;
        let vfs = crate::kernel::vfs::mounted().ok_or(Error::NoDev)?;
        let (node, canonical_path) = vfs.resolve_with_path_blocking(&path, &process.credentials()?)?;
        if !matches!(node, Node::Directory { .. }) {
            return Err(Error::NotDir);
        }
        process.set_current_directory(canonical_path.as_bytes())?;
        Ok(0)
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/chdir.rs"]
mod tests;