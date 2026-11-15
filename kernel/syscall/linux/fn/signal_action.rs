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
use crate::kernel::signal::{SignalAction, SignalSet, MAX_SIGNAL_NUMBER, SA_NOCLDSTOP};

const SIGSET_SIZE: u64 = 8;
const SA_SIGINFO: u64 = 0x0000_0004;
const SA_RESTORER: u64 = 0x0400_0000;
const SA_RESTART: u64 = 0x1000_0000;
const SA_NODEFER: u64 = 0x4000_0000;
const SA_RESETHAND: u64 = 0x8000_0000;
const SUPPORTED_FLAGS: u64 = SA_NOCLDSTOP as u64
    | SA_SIGINFO
    | SA_RESTART
    | SA_NODEFER
    | SA_RESETHAND;
const UNSUPPORTED_FLAGS: u64 = 0x0000_0002 | SA_RESTORER | 0x0800_0000;

pub(crate) struct RtSigaction;

impl RtSigaction {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::RtSigaction;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        if frame.arguments[3] != SIGSET_SIZE {
            return Err(Error::Inval);
        }
        let signal_number = u8::try_from(frame.arguments[0]).map_err(|_| Error::Inval)?;
        if signal_number == 0 || signal_number > MAX_SIGNAL_NUMBER || matches!(signal_number, 9 | 19) {
            return Err(Error::Inval);
        }
        let process = ThreadManager::current_process()?;
        let mut address_space = process.address_space();
        let new_action = if frame.arguments[1] == 0 {
            None
        } else {
            let address = usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?;
            let mut bytes = [0; 32];
            address_space.read(address, &mut bytes).map_err(|_| Error::Fault)?;
            Some(decode_action(&bytes)?)
        };
        let previous = process.signal_action(signal_number)?;
        if frame.arguments[2] != 0 {
            let address = usize::try_from(frame.arguments[2]).map_err(|_| Error::Fault)?;
            address_space
                .write(address, &encode_action(previous))
                .map_err(|_| Error::Fault)?;
        }
        if let Some(action) = new_action {
            process.set_signal_action(signal_number, action)?;
        }
        Ok(0)
    }
}

fn decode_action(bytes: &[u8; 32]) -> Result<SignalAction, Error> {
    let handler = usize::try_from(read_word(bytes, 0)).map_err(|_| Error::Fault)?;
    let raw_flags = read_word(bytes, 8);
    if raw_flags & !(SUPPORTED_FLAGS | UNSUPPORTED_FLAGS) != 0 {
        return Err(Error::Inval);
    }
    if raw_flags & UNSUPPORTED_FLAGS != 0 {
        return Err(Error::NoSys);
    }
    let mask = SignalSet::from_bits(read_word(bytes, 24));
    Ok(SignalAction {
        handler,
        mask,
        flags: (raw_flags & !SA_RESTORER) as u32,
    })
}

fn encode_action(action: SignalAction) -> [u8; 32] {
    let mut output = [0; 32];
    output[..8].copy_from_slice(&(action.handler as u64).to_ne_bytes());
    output[8..16].copy_from_slice(&u64::from(action.flags).to_ne_bytes());
    output[16..24].copy_from_slice(&0u64.to_ne_bytes());
    output[24..32].copy_from_slice(&action.mask.bits().to_ne_bytes());
    output
}

fn read_word(bytes: &[u8; 32], offset: usize) -> u64 {
    u64::from_ne_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/signal_action.rs"]
mod tests;