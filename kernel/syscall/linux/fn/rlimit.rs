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

const RLIMIT_NLIMITS: i32 = 16;

pub(crate) struct Getrlimit;
pub(crate) struct Setrlimit;

fn kernel_resource(resource: i32) -> Result<i32, Error> {
    match resource {
        0..=4 | 9 => Ok(resource),
        7 => Ok(5),
        5..=8 | 10..RLIMIT_NLIMITS => Err(Error::NoSys),
        _ => Err(Error::Inval),
    }
}

fn encode_limits(soft: u64, hard: u64) -> [u8; 16] {
    let mut output = [0; 16];
    output[..8].copy_from_slice(&soft.to_ne_bytes());
    output[8..].copy_from_slice(&hard.to_ne_bytes());
    output
}

fn decode_limits(bytes: &[u8; 16]) -> (u64, u64) {
    (
        u64::from_ne_bytes(bytes[..8].try_into().unwrap()),
        u64::from_ne_bytes(bytes[8..].try_into().unwrap()),
    )
}

impl Getrlimit {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Getrlimit;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let resource = kernel_resource(frame.arguments[0] as i32)?;
        let address = usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?;
        if address == 0 {
            return Err(Error::Fault);
        }
        let process = ThreadManager::current_process()?;
        let (soft, hard) = process.resource_limit(resource)?;
        let mut address_space = process.address_space();
        address_space
            .write(address, &encode_limits(soft, hard))
            .map_err(|_| Error::Fault)?;
        Ok(0)
    }
}

impl Setrlimit {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Setrlimit;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let resource = kernel_resource(frame.arguments[0] as i32)?;
        let address = usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?;
        if address == 0 {
            return Err(Error::Fault);
        }
        let process = ThreadManager::current_process()?;
        let mut bytes = [0; 16];
        process
            .address_space()
            .read(address, &mut bytes)
            .map_err(|_| Error::Fault)?;
        let (soft, hard) = decode_limits(&bytes);
        process.set_resource_limit(resource, soft, hard)?;
        Ok(0)
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/rlimit.rs"]
mod tests;