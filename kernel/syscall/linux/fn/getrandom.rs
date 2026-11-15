/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

const GRND_NONBLOCK: u32 = 0x0001;
const GRND_RANDOM: u32 = 0x0002;
const GRND_INSECURE: u32 = 0x0004;
const SUPPORTED_FLAGS: u32 = GRND_NONBLOCK | GRND_RANDOM | GRND_INSECURE;
const MAX_GETRANDOM_LENGTH: usize = 32 * 1024 * 1024;
const CHUNK_LENGTH: usize = 256;

pub(crate) struct Getrandom;

impl Getrandom {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Getrandom;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let flags = frame.arguments[2] as u32;
        if flags & !SUPPORTED_FLAGS != 0 {
            return Err(Error::Inval);
        }
        let requested = usize::try_from(frame.arguments[1]).map_err(|_| Error::Fbig)?;
        let length = requested_length(requested);
        if length == 0 {
            return Ok(0);
        }
        let address = usize::try_from(frame.arguments[0]).map_err(|_| Error::Fault)?;
        if address == 0 {
            return Err(Error::Fault);
        }
        let process = ThreadManager::current_process()?;
        let mut address_space = process.address_space();
        let mut buffer = [0; CHUNK_LENGTH];
        let mut written = 0usize;
        while written < length {
            let chunk_length = CHUNK_LENGTH.min(length - written);
            if let Err(error) = crate::random::fill(&mut buffer[..chunk_length]) {
                return if written == 0 {
                    Err(error)
                } else {
                    i64::try_from(written).map_err(|_| Error::Overflow)
                };
            }
            let Some(destination) = address.checked_add(written) else {
                return if written == 0 {
                    Err(Error::Fault)
                } else {
                    i64::try_from(written).map_err(|_| Error::Overflow)
                };
            };
            if address_space
                .write(destination, &buffer[..chunk_length])
                .is_err()
            {
                return if written == 0 {
                    Err(Error::Fault)
                } else {
                    i64::try_from(written).map_err(|_| Error::Overflow)
                };
            }
            written += chunk_length;
        }
        i64::try_from(written).map_err(|_| Error::Overflow)
    }
}

fn requested_length(requested: usize) -> usize {
    requested.min(MAX_GETRANDOM_LENGTH)
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/getrandom.rs"]
mod tests;