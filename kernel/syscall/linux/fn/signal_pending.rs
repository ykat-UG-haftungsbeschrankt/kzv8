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
use crate::kernel::signal::SignalSet;

const SIGSET_SIZE: u64 = 8;

pub(crate) struct RtSigpending;

impl RtSigpending {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::RtSigpending;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        if frame.arguments[1] != SIGSET_SIZE {
            return Err(Error::Inval);
        }
        let address = usize::try_from(frame.arguments[0]).map_err(|_| Error::Fault)?;
        if address == 0 {
            return Err(Error::Fault);
        }
        let process = ThreadManager::current_process()?;
        let thread_pending = ThreadManager::pending_current_signals()?;
        let process_pending = process.pending_signals();
        let blocked = ThreadManager::current_signal_context()?.1;
        let output = pending_blocked_mask(thread_pending, process_pending, blocked).to_ne_bytes();
        let mut address_space = process.address_space();
        address_space.write(address, &output).map_err(|_| Error::Fault)?;
        Ok(0)
    }
}

fn pending_blocked_mask(
    thread_pending: SignalSet,
    process_pending: SignalSet,
    blocked: SignalSet,
) -> u64 {
    thread_pending.union(process_pending).intersection(blocked).bits()
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/signal_pending.rs"]
mod tests;