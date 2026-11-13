/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::error::Error;
use crate::kernel::dl::AddressSpace;
use super::super::SyscallFrame;
use crate::kernel::vfs::FileHandleApi as _;

const MAX_WRITE_LENGTH: usize = 1 << 20;

pub(crate) struct Write;

pub(crate) struct WriteArguments {
    pub(crate) file_descriptor: u64,
    pub(crate) address: u64,
    pub(crate) length: u64,
}

impl Write {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Write;

    pub(crate) fn callback(arguments: WriteArguments) -> Result<i64, Error> {
        Self::write_file(arguments)
    }

    fn write_file(arguments: WriteArguments) -> Result<i64, Error> {
        let file_descriptor =
            u32::try_from(arguments.file_descriptor).map_err(|_| Error::Badf)?;
        let length = usize::try_from(arguments.length).map_err(|_| Error::Fbig)?;
        if length > MAX_WRITE_LENGTH {
            return Err(Error::Fbig);
        }
        let process = crate::kernel::scheduler::thread::ThreadManager::current_process()?;
        let file = process.file_descriptor(file_descriptor)?;
        if length == 0 {
            return Ok(0);
        }
        let address = usize::try_from(arguments.address).map_err(|_| Error::Fault)?;
        let address_space = process.address_space();
        let mut file_handle = file.take_file()?;
        let mut buffer = [0u8; 256];
        let mut written = 0usize;
        let result = (|| -> Result<i64, Error> {
            while written < length {
                let chunk_length = buffer.len().min(length - written);
                let Some(source) = address.checked_add(written) else {
                    return if written == 0 {
                        Err(Error::Fault)
                    } else {
                        Ok(written as i64)
                    };
                };
                if address_space
                    .read(source, &mut buffer[..chunk_length])
                    .is_err()
                {
                    return if written == 0 {
                        Err(Error::Fault)
                    } else {
                        Ok(written as i64)
                    };
                }
                match file_handle.write_blocking(&buffer[..chunk_length]) {
                    Ok(0) => break,
                    Ok(written_chunk) if written_chunk <= chunk_length => {
                        written += written_chunk;
                        if written_chunk < chunk_length {
                            break;
                        }
                    }
                    Ok(_) => {
                        return if written == 0 {
                            Err(Error::Io)
                        } else {
                            Ok(written as i64)
                        };
                    }
                    Err(_) if written != 0 => return Ok(written as i64),
                    Err(error) => return Err(error),
                }
            }
            Ok(written as i64)
        })();
        file.restore_file(file_handle)?;
        result
    }

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        Self::callback(WriteArguments {
            file_descriptor: frame.arguments[0],
            address: frame.arguments[1],
            length: frame.arguments[2],
        })
    }
}