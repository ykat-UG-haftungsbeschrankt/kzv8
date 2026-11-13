/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::PAGE_SIZE;
use super::super::SyscallFrame;
use crate::kernel::error::Error;

pub(crate) struct Munmap;

pub(crate) struct MunmapArguments {
    pub(crate) address: u64,
    pub(crate) length: u64,
}

impl Munmap {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Munmap;

    pub(crate) fn callback(arguments: MunmapArguments) -> Result<i64, Error> {
        let Ok(address) = usize::try_from(arguments.address) else {
            return Err(Error::Inval);
        };
        let Ok(length) = usize::try_from(arguments.length) else {
            return Err(Error::Inval);
        };
        if address == 0 || address % PAGE_SIZE != 0 || length == 0 {
            return Err(Error::Inval);
        }
        let process = crate::kernel::scheduler::thread::ThreadManager::current_process()?;
        let Some((released_bytes, resident_bytes)) = process
            .address_space()
            .release_user_mapping_accounted(address, length)
        else {
            return Err(Error::Inval);
        };
        process.release_memory(released_bytes, released_bytes, resident_bytes);
        Ok(0)
    }

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        Self::callback(MunmapArguments {
            address: frame.arguments[0],
            length: frame.arguments[1],
        })
    }
}