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

const SIG_BLOCK: i32 = 0;
const SIG_UNBLOCK: i32 = 1;
const SIG_SETMASK: i32 = 2;
const SIGSET_SIZE: u64 = 8;

pub(crate) struct RtSigprocmask;

impl RtSigprocmask {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::RtSigprocmask;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        if frame.arguments[3] != SIGSET_SIZE {
            return Err(Error::Inval);
        }
        let process = ThreadManager::current_process()?;
        let current = ThreadManager::current_signal_context()?.1;
        let mut address_space = process.address_space();
        let requested = if frame.arguments[1] == 0 {
            None
        } else {
            let address = usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?;
            let mut bytes = [0; 8];
            address_space.read(address, &mut bytes).map_err(|_| Error::Fault)?;
            Some(SignalSet::from_bits(u64::from_ne_bytes(bytes)))
        };
        let old_address = if frame.arguments[2] == 0 {
            None
        } else {
            Some(usize::try_from(frame.arguments[2]).map_err(|_| Error::Fault)?)
        };
        let updated = match requested {
            Some(requested) => Some(update_mask(current, frame.arguments[0] as i32, requested)?),
            None => None,
        };
        if let Some(address) = old_address {
            address_space
                .write(address, &current.bits().to_ne_bytes())
                .map_err(|_| Error::Fault)?;
        }
        if let Some(updated) = updated {
            ThreadManager::set_current_signal_mask(updated)?;
        }
        Ok(0)
    }
}

fn update_mask(current: SignalSet, how: i32, requested: SignalSet) -> Result<SignalSet, Error> {
    let updated = match how {
        SIG_BLOCK => current.union(requested),
        SIG_UNBLOCK => current.difference(requested),
        SIG_SETMASK => requested,
        _ => return Err(Error::Inval),
    };
    Ok(updated.without_uncatchable())
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/signal_mask.rs"]
mod tests;