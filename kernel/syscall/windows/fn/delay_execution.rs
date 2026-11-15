/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::arch::traits::TimerArchitecture as _;
use crate::kernel::dl::AddressSpace as _;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct DelayExecution;

impl DelayExecution {
    pub(crate) const ID: u64 = 0x0034;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<u32, Error> {
        if frame.windows_arguments[0] != 0 {
            return Err(Error::OpNotSupp);
        }
        let interval_address = usize::try_from(frame.windows_arguments[1])
            .map_err(|_| Error::Fault)?;
        let process = ThreadManager::current_process()?;
        let mut bytes = [0; core::mem::size_of::<i64>()];
        process
            .address_space()
            .read(interval_address, &mut bytes)
            .map_err(|_| Error::Fault)?;
        let interval = i64::from_le_bytes(bytes);
        if interval == 0 {
            return Ok(0);
        }
        if interval > 0 {
            return Err(Error::NoSys);
        }
        let relative_interval = interval.checked_neg().ok_or(Error::Inval)?;
        let delay_ns = u64::try_from(relative_interval)
            .map_err(|_| Error::Overflow)?
            .checked_mul(100)
            .ok_or(Error::Overflow)?;
        if delay_ns != 0 {
            let now_ns = crate::kernel::arch::TimerArchitecture::now_ns()
                .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns);
            let deadline_ns = now_ns.checked_add(delay_ns).ok_or(Error::Overflow)?;
            let _slept = ThreadManager::sleep_current_until(deadline_ns);
        }
        Ok(0)
    }
}