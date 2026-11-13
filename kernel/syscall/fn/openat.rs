/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::string::String;
use alloc::vec::Vec;

use super::super::SyscallFrame;
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::scheduler::thread::vm::UserAddressSpace;
use crate::kernel::vfs::{OpenMode, VfsApi as _};

const AT_FDCWD: i64 = -100;
const OPEN_ACCESS_MODE_MASK: u64 = 0x3;
const MAX_PATH_LENGTH: usize = 4096;
const PATH_COPY_CHUNK: usize = 64;

pub(crate) struct OpenAt;

pub(crate) struct OpenAtArguments {
    pub(crate) directory_file_descriptor: u64,
    pub(crate) path_address: u64,
    pub(crate) flags: u64,
    pub(crate) mode: u64,
}

impl OpenAt {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::OpenAt;

    pub(crate) fn callback(arguments: OpenAtArguments) -> Result<i64, Error> {
        if arguments.directory_file_descriptor as i64 != AT_FDCWD {
            return Err(Error::Badf);
        }
        if arguments.flags & !OPEN_ACCESS_MODE_MASK != 0 {
            return Err(Error::Inval);
        }
        let open_mode = match arguments.flags & OPEN_ACCESS_MODE_MASK {
            0 => OpenMode::ReadOnly,
            1 => OpenMode::WriteOnly,
            2 => OpenMode::ReadWrite,
            _ => return Err(Error::Inval),
        };
        let path_address = usize::try_from(arguments.path_address).map_err(|_| Error::Fault)?;
        let process = ThreadManager::current_process()?;
        let address_space = process.address_space();
        let path = copy_user_path(&address_space, path_address)?;
        let vfs = crate::kernel::vfs::mounted().ok_or(Error::NoDev)?;
        let file = vfs.open_blocking(&path, open_mode, process.credentials()?)?;
        let descriptor = process.insert_file_descriptor(file)?;
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

fn copy_user_path(address_space: &UserAddressSpace, address: usize) -> Result<String, Error> {
    if address == 0 {
        return Err(Error::Fault);
    }
    let mut path = Vec::new();
    path.try_reserve(128).map_err(|_| Error::NoMem)?;
    while path.len() < MAX_PATH_LENGTH {
        let current = address.checked_add(path.len()).ok_or(Error::Fault)?;
        let page_remaining = crate::kernel::memory::r#virtual::PAGE_SIZE
            - current % crate::kernel::memory::r#virtual::PAGE_SIZE;
        let chunk_length = PATH_COPY_CHUNK
            .min(page_remaining)
            .min(MAX_PATH_LENGTH - path.len());
        let mut chunk = [0; PATH_COPY_CHUNK];
        address_space
            .read(current, &mut chunk[..chunk_length])
            .map_err(|_| Error::Fault)?;
        let length = chunk[..chunk_length]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(chunk_length);
        path.try_reserve(length).map_err(|_| Error::NoMem)?;
        path.extend_from_slice(&chunk[..length]);
        if length < chunk_length {
            return String::from_utf8(path).map_err(|_| Error::Inval);
        }
    }
    Err(Error::NameTooLong)
}