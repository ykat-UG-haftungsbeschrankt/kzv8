/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::dl::AddressSpace as _;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

const FUTEX_CMD_MASK: u32 = 0x7f;
const FUTEX_PRIVATE_FLAG: u32 = 0x80;
const FUTEX_CLOCK_REALTIME: u32 = 0x100;
const FUTEX_VALID_FLAGS: u32 = FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME;
const FUTEX_WAIT: u32 = 0;
const FUTEX_WAKE: u32 = 1;
const FUTEX_WAIT_BITSET: u32 = 9;
const FUTEX_WAKE_BITSET: u32 = 10;

pub(crate) struct Futex;

impl Futex {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Futex;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let address = usize::try_from(frame.arguments[0]).map_err(|_| Error::Fault)?;
        if address == 0 || address % core::mem::align_of::<u32>() != 0 {
            return Err(Error::Inval);
        }
        let operation = decode_operation(frame.arguments[1])?;
        let process = ThreadManager::current_process()?;
        let address_space = process.address_space();
        let mut value = [0; core::mem::size_of::<u32>()];
        address_space
            .read(address, &mut value)
            .map_err(|_| Error::Fault)?;

        match operation {
            FUTEX_WAKE => Ok(0),
            FUTEX_WAKE_BITSET => {
                if frame.arguments[5] == 0 {
                    Err(Error::Inval)
                } else {
                    Ok(0)
                }
            }
            FUTEX_WAIT => check_wait_value(value, frame.arguments[2]),
            FUTEX_WAIT_BITSET => {
                if frame.arguments[5] == 0 {
                    Err(Error::Inval)
                } else {
                    check_wait_value(value, frame.arguments[2])
                }
            }
            _ => Err(Error::NoSys),
        }
    }
}

fn decode_operation(operation: u64) -> Result<u32, Error> {
    let operation = u32::try_from(operation).map_err(|_| Error::Inval)?;
    if operation & !(FUTEX_CMD_MASK | FUTEX_VALID_FLAGS) != 0 {
        return Err(Error::Inval);
    }
    Ok(operation & FUTEX_CMD_MASK)
}

fn check_wait_value(value: [u8; core::mem::size_of::<u32>()], expected: u64) -> Result<i64, Error> {
    let expected = u32::try_from(expected).map_err(|_| Error::Inval)?;
    let value = u32::from_ne_bytes(value);
    if value != expected {
        return Err(Error::Again);
    }
    Ok(0)
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/futex.rs"]
mod tests;