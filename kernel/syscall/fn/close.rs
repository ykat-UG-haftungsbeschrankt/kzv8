/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct Close;

pub(crate) struct CloseArguments {
    pub(crate) file_descriptor: u64,
}

impl Close {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Close;

    pub(crate) fn callback(arguments: CloseArguments) -> Result<i64, Error> {
        let file_descriptor =
            u32::try_from(arguments.file_descriptor).map_err(|_| Error::Badf)?;
        ThreadManager::current_process()?.close_file_descriptor(file_descriptor)?;
        Ok(0)
    }

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        Self::callback(CloseArguments {
            file_descriptor: frame.arguments[0],
        })
    }
}