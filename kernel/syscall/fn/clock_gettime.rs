/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::arch::traits::TimerArchitecture as _;
use crate::kernel::error::Error;
use super::super::SyscallFrame;

#[repr(C)]
struct UserTimespec {
    seconds: i64,
    nanoseconds: i64,
}

pub(crate) struct ClockGetTime;

pub(crate) struct ClockGetTimeArguments {
    pub(crate) clock_id: u64,
    pub(crate) output_address: u64,
}

impl ClockGetTime {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::ClockGetTime;

    pub(crate) fn callback(arguments: ClockGetTimeArguments) -> Result<i64, Error> {
        if !matches!(arguments.clock_id, 1 | 4) {
            return Err(Error::Inval);
        }
        let Ok(output_address) = usize::try_from(arguments.output_address) else {
            return Err(Error::Fault);
        };
        if !crate::kernel::memory::r#virtual::user_range_accessible(
            output_address,
            core::mem::size_of::<UserTimespec>(),
            true,
        ) {
            return Err(Error::Fault);
        }
        let now = crate::kernel::arch::TimerArchitecture::now_ns()
            .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns);
        let value = UserTimespec {
            seconds: (now / 1_000_000_000) as i64,
            nanoseconds: (now % 1_000_000_000) as i64,
        };
        unsafe {
            core::ptr::write_unaligned(output_address as *mut UserTimespec, value);
        }
        Ok(0)
    }

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        Self::callback(ClockGetTimeArguments {
            clock_id: frame.arguments[0],
            output_address: frame.arguments[1],
        })
    }
}