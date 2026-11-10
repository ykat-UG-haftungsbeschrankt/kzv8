/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::error::Error;
use super::super::SyscallFrame;

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
        if arguments.file_descriptor != 1 && arguments.file_descriptor != 2 {
            return Err(Error::Badf);
        }
        let Ok(length) = usize::try_from(arguments.length) else {
            return Err(Error::Fbig);
        };
        if length > MAX_WRITE_LENGTH {
            return Err(Error::Fbig);
        }
        if length == 0 {
            return Ok(0);
        }
        let Ok(address) = usize::try_from(arguments.address) else {
            return Err(Error::Fault);
        };

        let mut buffer = [0u8; 256];
        let mut written = 0usize;
        while written < length {
            let count = buffer.len().min(length - written);
            let Some(source) = address.checked_add(written) else {
                return if written == 0 {
                    Err(Error::Fault)
                } else {
                    Ok(written as i64)
                };
            };
            if !crate::kernel::memory::r#virtual::user_range_accessible(source, count, false) {
                return if written == 0 {
                    Err(Error::Fault)
                } else {
                    Ok(written as i64)
                };
            }
            unsafe {
                core::ptr::copy_nonoverlapping(source as *const u8, buffer.as_mut_ptr(), count);
            }
            if !crate::kernel::driver::virtio::console::Console::write(&buffer[..count]) {
                return if written == 0 {
                    Err(Error::Io)
                } else {
                    Ok(written as i64)
                };
            }
            written += count;
        }
        Ok(written as i64)
    }

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        Self::callback(WriteArguments {
            file_descriptor: frame.arguments[0],
            address: frame.arguments[1],
            length: frame.arguments[2],
        })
    }
}