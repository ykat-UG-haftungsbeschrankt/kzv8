/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::vfs::VfsApi as _;

pub(crate) struct Sync;

impl Sync {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Sync;

    pub(crate) fn handle(_frame: SyscallFrame) -> Result<i64, Error> {
        if let Some(vfs) = crate::kernel::vfs::mounted() {
            vfs.sync_filesystems_blocking()?;
        }
        Ok(0)
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/sync.rs"]
mod tests;