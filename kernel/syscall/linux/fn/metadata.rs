/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;

use super::super::SyscallFrame;
use super::path::{copy_user_path, resolve_at_current_directory};
use crate::kernel::credentials::{GroupId, UserId};
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::vfs::{FileHandleApi as _, NodeApi as _, NodeMetadata, NodeMetadataApi as _, VfsApi as _};

const PERMISSION_BITS: u32 = 0o7777;

pub(crate) struct Chmod;
pub(crate) struct Fchmod;
pub(crate) struct Chown;
pub(crate) struct Fchown;

fn chmod_mode(mode: u64) -> u16 {
    (mode as u32 & PERMISSION_BITS) as u16
}

fn update_owner(
    metadata: &NodeMetadata,
    user: u64,
    group: u64,
    credentials: &crate::kernel::process::Credentials,
) -> Result<(), Error> {
    let user = if user as u32 == u32::MAX {
        metadata.owner()
    } else {
        UserId::new(user as u32)
    };
    let group = if group as u32 == u32::MAX {
        metadata.group()
    } else {
        GroupId::new(group as u32)
    };
    metadata.set_owner_and_group(credentials, user, group)
}

fn path_metadata(
    frame: SyscallFrame,
) -> Result<(NodeMetadata, Arc<crate::kernel::process::Process>), Error> {
    let process = ThreadManager::current_process()?;
    let address_space = process.address_space();
    let user_path = copy_user_path(
        &address_space,
        usize::try_from(frame.arguments[0]).map_err(|_| Error::Fault)?,
    )?;
    let path = resolve_at_current_directory(user_path, &process.current_directory()?)?;
    let vfs = crate::kernel::vfs::mounted().ok_or(Error::NoDev)?;
    let (node, _) = vfs.resolve_with_path_blocking(&path, &process.credentials()?)?;
    Ok((node.metadata(), process))
}

fn descriptor_metadata(
    file_descriptor: u64,
) -> Result<(NodeMetadata, Arc<crate::kernel::process::Process>), Error> {
    let descriptor = u32::try_from(file_descriptor).map_err(|_| Error::Badf)?;
    let process = ThreadManager::current_process()?;
    let object = process.file_descriptor(descriptor)?;
    let mut file = object.take_file()?;
    let metadata = file.metadata();
    object.restore_file(file)?;
    Ok((metadata, process))
}

impl Chmod {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Chmod;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let (metadata, process) = path_metadata(frame)?;
        metadata.set_mode(&process.credentials()?, chmod_mode(frame.arguments[1]))?;
        Ok(0)
    }
}

impl Fchmod {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Fchmod;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let descriptor = u32::try_from(frame.arguments[0]).map_err(|_| Error::Badf)?;
        let process = ThreadManager::current_process()?;
        let object = process.file_descriptor(descriptor)?;
        let mut file = object.take_file()?;
        let result = file
            .metadata()
            .set_mode(&process.credentials()?, chmod_mode(frame.arguments[1]));
        object.restore_file(file)?;
        result?;
        Ok(0)
    }
}

impl Chown {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Chown;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let (metadata, process) = path_metadata(frame)?;
        update_owner(
            &metadata,
            frame.arguments[1],
            frame.arguments[2],
            &process.credentials()?,
        )?;
        Ok(0)
    }
}

impl Fchown {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Fchown;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let (metadata, process) = descriptor_metadata(frame.arguments[0])?;
        update_owner(
            &metadata,
            frame.arguments[1],
            frame.arguments[2],
            &process.credentials()?,
        )?;
        Ok(0)
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/metadata.rs"]
mod tests;