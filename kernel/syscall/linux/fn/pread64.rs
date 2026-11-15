/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use super::super::SyscallFrame;
use crate::kernel::dl::AddressSpace as _;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::vfs::{FileHandleApi as _, SeekFrom};

const MAX_PREAD_LENGTH: usize = 1 << 20;

pub(crate) struct Pread64;

impl Pread64 {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Pread64;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let file_descriptor = u32::try_from(frame.arguments[0]).map_err(|_| Error::Badf)?;
        let address = usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?;
        let length = usize::try_from(frame.arguments[2]).map_err(|_| Error::Fbig)?;
        let offset = frame.arguments[3];
        crate::kernel::debug::debug_fmt(format_args!(
            "kz: pread64 fd={} len={} offset={}\n",
            file_descriptor,
            length,
            offset
        ));
        validate_arguments(length, offset)?;
        if length == 0 {
            return Ok(0);
        }
        if file_descriptor < 3 {
            return Err(if file_descriptor == 0 {
                Error::Spipe
            } else {
                Error::Badf
            });
        }

        let process = ThreadManager::current_process()?;
        let mut address_space = process.address_space();
        if !address_space.user_range_accessible(address, length, true) {
            return Err(Error::Fault);
        }
        let object = process.file_descriptor(file_descriptor)?;
        let mut file = object.take_file()?;
        let original_position = file.position();
        let mut buffer = Vec::new();
        let result = (|| {
            buffer.try_reserve_exact(length).map_err(|_| Error::NoMem)?;
            buffer.resize(length, 0);
            file.seek(SeekFrom::Start(offset))?;
            let count = file.read_blocking(&mut buffer)?;
            if count > buffer.len() {
                return Err(Error::Io);
            }
            Ok(count)
        })();
        let restore_result = file.seek(SeekFrom::Start(original_position));
        object.restore_file(file)?;
        let count = result?;
        restore_result?;
        if count != 0 {
            address_space
                .write(address, &buffer[..count])
                .map_err(|_| Error::Fault)?;
        }
        i64::try_from(count).map_err(|_| Error::Overflow)
    }
}

fn validate_arguments(length: usize, offset: u64) -> Result<(), Error> {
    if length > MAX_PREAD_LENGTH {
        return Err(Error::Fbig);
    }
    if offset > i64::MAX as u64 {
        return Err(Error::Inval);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/pread64.rs"]
mod tests;