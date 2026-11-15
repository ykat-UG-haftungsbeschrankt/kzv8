/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::arch::traits::TimerArchitecture as _;
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;

pub(crate) struct Nanosleep;

impl Nanosleep {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Nanosleep;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let request_address = usize::try_from(frame.arguments[0]).map_err(|_| Error::Fault)?;
        if request_address == 0 {
            return Err(Error::Fault);
        }
        let process = ThreadManager::current_process()?;
        let mut address_space = process.address_space();
        let mut request_bytes = [0; 16];
        address_space
            .read(request_address, &mut request_bytes)
            .map_err(|_| Error::Fault)?;
        let seconds = i64::from_ne_bytes(request_bytes[..8].try_into().map_err(|_| Error::Fault)?);
        let nanoseconds =
            i64::from_ne_bytes(request_bytes[8..].try_into().map_err(|_| Error::Fault)?);
        let duration = duration_ns(seconds, nanoseconds)?;
        if duration == 0 {
            return Ok(0);
        }
        let now = crate::kernel::arch::TimerArchitecture::now_ns()
            .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns);
        let deadline = now.checked_add(duration).unwrap_or(u64::MAX);
        if !ThreadManager::sleep_current_until(deadline) {
            return Ok(0);
        }
        let remaining_address = usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?;
        if remaining_address != 0 {
            let now = crate::kernel::arch::TimerArchitecture::now_ns()
                .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns);
            let remaining = deadline.saturating_sub(now);
            let output = timespec_bytes(remaining);
            address_space
                .write(remaining_address, &output)
                .map_err(|_| Error::Fault)?;
        }
        Err(Error::Intr)
    }
}

fn duration_ns(seconds: i64, nanoseconds: i64) -> Result<u64, Error> {
    if seconds < 0 || !(0..1_000_000_000).contains(&nanoseconds) {
        return Err(Error::Inval);
    }
    u64::try_from(seconds)
        .map_err(|_| Error::Inval)?
        .checked_mul(NANOSECONDS_PER_SECOND)
        .and_then(|value| value.checked_add(nanoseconds as u64))
        .ok_or(Error::Inval)
}

fn timespec_bytes(duration_ns: u64) -> [u8; 16] {
    let seconds = (duration_ns / NANOSECONDS_PER_SECOND) as i64;
    let nanoseconds = (duration_ns % NANOSECONDS_PER_SECOND) as i64;
    let mut output = [0; 16];
    output[..8].copy_from_slice(&seconds.to_ne_bytes());
    output[8..].copy_from_slice(&nanoseconds.to_ne_bytes());
    output
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/nanosleep.rs"]
mod tests;