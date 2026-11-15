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
use crate::kernel::vfs::{FileHandleApi as _, OpenMode, VfsApi as _};

const OPEN_ACCESS_MODE_MASK: u64 = 0x3;
const OPEN_DIRECTORY: u64 = 0x1_0000;
const OPEN_CLOEXEC: u64 = 0x8_0000;
const OPEN_IGNORED_FLAGS: u64 = 0o400 | 0o100000;
const OPEN_UNSUPPORTED_FLAGS: u64 = 0o100
    | 0o200
    | 0o1000
    | 0o2000
    | 0o4000
    | 0o10000
    | 0o20000
    | 0o40000
    | 0o400000
    | 0o1000000
    | 0o4000000
    | 0o10000000
    | 0o20000000;
const OPEN_KNOWN_FLAGS: u64 = OPEN_ACCESS_MODE_MASK
    | OPEN_DIRECTORY
    | OPEN_CLOEXEC
    | OPEN_IGNORED_FLAGS
    | OPEN_UNSUPPORTED_FLAGS;

pub(crate) struct OpenAt;
#[cfg(target_arch = "x86_64")]
pub(crate) struct Open;

pub(crate) struct OpenAtArguments {
    pub(crate) directory_file_descriptor: u64,
    pub(crate) path_address: u64,
    pub(crate) flags: u64,
    pub(crate) mode: u64,
}

impl OpenAt {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::OpenAt;

    pub(crate) fn callback(arguments: OpenAtArguments) -> Result<i64, Error> {
        validate_flags(arguments.flags)?;
        let open_mode = match arguments.flags & OPEN_ACCESS_MODE_MASK {
            0 => OpenMode::ReadOnly,
            1 => OpenMode::WriteOnly,
            2 => OpenMode::ReadWrite,
            _ => return Err(Error::Inval),
        };
        let path_address = usize::try_from(arguments.path_address).map_err(|_| Error::Fault)?;
        let process = ThreadManager::current_process()?;
        let address_space = process.address_space();
        let path = resolve_at_directory(
            copy_user_path(&address_space, path_address)?,
            arguments.directory_file_descriptor,
            &process,
        )?;
        let vfs = crate::kernel::vfs::mounted().ok_or(Error::NoDev)?;
        let file = vfs.open_blocking(&path, open_mode, process.credentials()?)?;
        if arguments.flags & OPEN_DIRECTORY != 0 && !file.is_directory() {
            return Err(Error::NotDir);
        }
        let (descriptor_limit, _) = process.resource_limit(5)?;
        let descriptor = process.insert_file_descriptor_with_flags(
            file,
            arguments.flags & OPEN_CLOEXEC != 0,
        )?;
        if !descriptor_is_allowed(descriptor, descriptor_limit) {
            process.close_file_descriptor(descriptor)?;
            return Err(Error::Mfile);
        }
        let _ = arguments.mode;
        Ok(i64::from(descriptor))
    }

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        Self::callback(OpenAtArguments {
            directory_file_descriptor: frame.arguments[0],
            path_address: frame.arguments[1],
            flags: frame.arguments[2],
            mode: frame.arguments[3],
        })
    }
}

#[cfg(target_arch = "x86_64")]
impl Open {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Open;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        OpenAt::callback(OpenAtArguments {
            directory_file_descriptor: (-100i64) as u64,
            path_address: frame.arguments[0],
            flags: frame.arguments[1],
            mode: frame.arguments[2],
        })
    }
}

fn validate_flags(flags: u64) -> Result<(), Error> {
    if flags & !OPEN_KNOWN_FLAGS != 0 {
        Err(Error::Inval)
    } else if flags & OPEN_UNSUPPORTED_FLAGS != 0 {
        Err(Error::NoSys)
    } else {
        Ok(())
    }
}

fn descriptor_is_allowed(descriptor: u32, limit: u64) -> bool {
    u64::from(descriptor) < limit
}