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

pub(crate) struct QueryPerformanceCounter;

impl QueryPerformanceCounter {
    pub(crate) const ID: u64 = 0x0031;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<u32, Error> {
        let counter_address = usize::try_from(frame.windows_arguments[0])
            .map_err(|_| Error::Fault)?;
        if counter_address == 0 {
            return Err(Error::Fault);
        }
        let frequency_address = usize::try_from(frame.windows_arguments[1])
            .map_err(|_| Error::Fault)?;
        let process = ThreadManager::current_process()?;
        let mut address_space = process.address_space();
        if !address_space.user_range_accessible(counter_address, 8, true)
            || (frequency_address != 0
                && !address_space.user_range_accessible(frequency_address, 8, true))
        {
            return Err(Error::Fault);
        }
        let counter = crate::kernel::arch::TimerArchitecture::now_ns()
            .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns);
        address_space
            .write(counter_address, &counter.to_le_bytes())
            .map_err(|_| Error::Fault)?;
        if frequency_address != 0 {
            address_space
                .write(frequency_address, &1_000_000_000u64.to_le_bytes())
                .map_err(|_| Error::Fault)?;
        }
        Ok(0)
    }
}