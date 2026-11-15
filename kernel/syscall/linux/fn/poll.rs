/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use super::super::SyscallFrame;
use crate::kernel::arch::traits::{CpuArchitecture as _, TimerArchitecture as _};
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

const POLL_FD_SIZE: usize = 8;
const MAX_POLL_FDS: usize = 1024;
const POLLIN: i16 = 0x0001;
const POLLOUT: i16 = 0x0004;
const POLLNVAL: i16 = 0x0020;

pub(crate) struct Poll;

impl Poll {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Poll;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let descriptor_count = usize::try_from(frame.arguments[1]).map_err(|_| Error::Inval)?;
        if descriptor_count > MAX_POLL_FDS {
            return Err(Error::Inval);
        }
        let timeout_ms = (frame.arguments[2] as u32) as i32;
        let deadline = poll_deadline(timeout_ms, monotonic_time_ns());
        let buffer_length = descriptor_count
            .checked_mul(POLL_FD_SIZE)
            .ok_or(Error::Overflow)?;
        let process = ThreadManager::current_process()?;
        let mut poll_fds = Vec::new();
        poll_fds
            .try_reserve_exact(buffer_length)
            .map_err(|_| Error::NoMem)?;
        poll_fds.resize(buffer_length, 0);
        let address = if descriptor_count == 0 {
            None
        } else {
            let address = usize::try_from(frame.arguments[0]).map_err(|_| Error::Fault)?;
            process
                .address_space()
                .read(address, &mut poll_fds)
                .map_err(|_| Error::Fault)?;
            Some(address)
        };

        loop {
            let mut ready_count = 0;
            for poll_fd in poll_fds.chunks_exact_mut(POLL_FD_SIZE) {
                let descriptor = i32::from_ne_bytes([
                    poll_fd[0], poll_fd[1], poll_fd[2], poll_fd[3],
                ]);
                let events = i16::from_ne_bytes([poll_fd[4], poll_fd[5]]);
                let file_valid = descriptor >= 3
                    && process.file_descriptor(descriptor as u32).is_ok();
                let revents = ready_events(
                    descriptor,
                    events,
                    descriptor == 0 && crate::kernel::tty::input_ready(),
                    descriptor > 0 && crate::kernel::tty::output_ready(),
                    file_valid,
                );
                poll_fd[6..8].copy_from_slice(&revents.to_ne_bytes());
                if revents != 0 {
                    ready_count += 1;
                }
            }

            if ready_count != 0
                || timeout_ms == 0
                || deadline.is_some_and(|deadline| monotonic_time_ns() >= deadline)
            {
                if let Some(address) = address {
                    process
                        .address_space()
                        .write(address, &poll_fds)
                        .map_err(|_| Error::Fault)?;
                }
                return Ok(ready_count);
            }
            if ThreadManager::current_signal_wake_pending() {
                return Err(Error::Intr);
            }
            crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
        }
    }
}

fn poll_deadline(timeout_ms: i32, now_ns: u64) -> Option<u64> {
    (timeout_ms >= 0).then(|| {
        now_ns.saturating_add((timeout_ms as u64).saturating_mul(1_000_000))
    })
}

fn monotonic_time_ns() -> u64 {
    crate::kernel::arch::TimerArchitecture::now_ns()
        .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns)
}

fn ready_events(
    descriptor: i32,
    events: i16,
    input_ready: bool,
    output_ready: bool,
    file_valid: bool,
) -> i16 {
    if descriptor < 0 {
        return 0;
    }
    if descriptor >= 3 {
        return if file_valid { 0 } else { POLLNVAL };
    }

    let mut revents = 0;
    if descriptor == 0 && input_ready {
        revents |= events & POLLIN;
    }
    if descriptor > 0 && output_ready {
        revents |= events & POLLOUT;
    }
    revents
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/poll.rs"]
mod tests;