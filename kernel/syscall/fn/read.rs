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
use crate::kernel::vfs::FileHandleApi as _;

const MAX_READ_LENGTH: usize = 1 << 20;

pub(crate) struct Read;

pub(crate) struct ReadArguments {
    pub(crate) file_descriptor: u64,
    pub(crate) address: u64,
    pub(crate) length: u64,
}

impl Read {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Read;

    pub(crate) fn callback(arguments: ReadArguments) -> Result<i64, Error> {
        let file_descriptor =
            u32::try_from(arguments.file_descriptor).map_err(|_| Error::Badf)?;
        let length = usize::try_from(arguments.length).map_err(|_| Error::Fbig)?;
        if length > MAX_READ_LENGTH {
            return Err(Error::Fbig);
        }
        let process = ThreadManager::current_process()?;
        let file = process.file_descriptor(file_descriptor)?;
        if length == 0 {
            return Ok(0);
        }
        let address = usize::try_from(arguments.address).map_err(|_| Error::Fault)?;
        let mut address_space = process.address_space();

        let mut buffer = Vec::new();
        buffer.try_reserve_exact(length).map_err(|_| Error::NoMem)?;
        buffer.resize(length, 0);
        let mut file_handle = file.take_file()?;
        let result = file_handle.read_blocking(&mut buffer);
        file.restore_file(file_handle)?;
        let count = result?;
        if count > buffer.len() {
            return Err(Error::Io);
        }
        if count != 0 {
            address_space
                .write(address, &buffer[..count])
                .map_err(|_| Error::Fault)?;
        }
        Ok(count as i64)
    }

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        Self::callback(ReadArguments {
            file_descriptor: frame.arguments[0],
            address: frame.arguments[1],
            length: frame.arguments[2],
        })
    }
}