/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::vfs::{FileHandleApi as _, SeekFrom};

pub(crate) struct Lseek;

impl Lseek {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Lseek;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let descriptor = u32::try_from(frame.arguments[0]).map_err(|_| Error::Badf)?;
        let process = ThreadManager::current_process()?;
        let object = process.file_descriptor(descriptor)?;
        let mut file = object.take_file()?;
        let result = (|| {
            let from = seek_from(frame.arguments[1] as i64, frame.arguments[2] as i32)?;
            let position = file.seek(from)?;
            i64::try_from(position).map_err(|_| Error::Overflow)
        })();
        object.restore_file(file)?;
        result
    }
}

fn seek_from(offset: i64, origin: i32) -> Result<SeekFrom, Error> {
    match origin {
        0 if offset >= 0 => Ok(SeekFrom::Start(offset as u64)),
        0 => Err(Error::Inval),
        1 => Ok(SeekFrom::Current(offset)),
        2 => Ok(SeekFrom::End(offset)),
        _ => Err(Error::Inval),
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/lseek.rs"]
mod tests;