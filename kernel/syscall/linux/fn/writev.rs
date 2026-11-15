/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use super::super::SyscallFrame;
use super::write::{Write, WriteArguments};
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

const IOVEC_SIZE: usize = 16;
const MAX_IOVEC_COUNT: usize = 1024;
const MAX_WRITEV_LENGTH: usize = 1 << 20;

pub(crate) struct Writev;

impl Writev {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Writev;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let count = usize::try_from(frame.arguments[2]).map_err(|_| Error::Inval)?;
        if count > MAX_IOVEC_COUNT {
            return Err(Error::Inval);
        }
        if count == 0 {
            return Ok(0);
        }

        let address = usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?;
        let table_length = count.checked_mul(IOVEC_SIZE).ok_or(Error::Inval)?;
        let process = ThreadManager::current_process()?;
        let address_space = process.address_space();
        let mut iovecs = Vec::new();
        iovecs
            .try_reserve_exact(table_length)
            .map_err(|_| Error::NoMem)?;
        iovecs.resize(table_length, 0);
        address_space
            .read(address, &mut iovecs)
            .map_err(|_| Error::Fault)?;

        let mut requested_total = 0usize;
        for iovec in iovecs.chunks_exact(IOVEC_SIZE) {
            let length = usize::try_from(u64::from_ne_bytes(
                iovec[8..].try_into().map_err(|_| Error::Fault)?,
            ))
            .map_err(|_| Error::Fbig)?;
            requested_total = requested_total.checked_add(length).ok_or(Error::Fbig)?;
            if requested_total > MAX_WRITEV_LENGTH {
                return Err(Error::Fbig);
            }
        }

        let mut written_total = 0usize;
        for iovec in iovecs.chunks_exact(IOVEC_SIZE) {
            let base = u64::from_ne_bytes(iovec[..8].try_into().map_err(|_| Error::Fault)?);
            let length = usize::try_from(u64::from_ne_bytes(
                iovec[8..].try_into().map_err(|_| Error::Fault)?,
            ))
            .map_err(|_| Error::Fbig)?;
            if length == 0 {
                continue;
            }

            match Write::callback(WriteArguments {
                file_descriptor: frame.arguments[0],
                address: base,
                length: u64::try_from(length).map_err(|_| Error::Fbig)?,
            }) {
                Ok(written) => {
                    let written = usize::try_from(written).map_err(|_| Error::Io)?;
                    if written > length {
                        return Err(Error::Io);
                    }
                    written_total = written_total.checked_add(written).ok_or(Error::Overflow)?;
                    if written < length {
                        break;
                    }
                }
                Err(_) if written_total != 0 => break,
                Err(error) => return Err(error),
            }
        }

        i64::try_from(written_total).map_err(|_| Error::Overflow)
    }
}