/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use super::getrusage::encode_usage;
use crate::kernel::arch::traits::CpuArchitecture as _;
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::process::ChildExitStatus;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::signal::{CLD_CONTINUED, CLD_EXITED, CLD_KILLED, CLD_STOPPED, SIGCHLD};

const WNOHANG: u32 = 0x0000_0001;
const WUNTRACED: u32 = 0x0000_0002;
const WCONTINUED: u32 = 0x0000_0008;
const VALID_OPTIONS: u32 = WNOHANG | WUNTRACED | WCONTINUED;
const WAIT_STATUS_LENGTH: usize = core::mem::size_of::<i32>();
const RUSAGE_LENGTH: usize = 18 * core::mem::size_of::<i64>();

#[derive(Clone, Copy, Debug)]
struct WaitOptions {
    no_hang: bool,
    stopped: bool,
    continued: bool,
}

pub(crate) struct Wait4;

impl Wait4 {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Wait4;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let process_id = frame.arguments[0] as u32 as i32;
        let status_address = usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?;
        let options = parse_options(frame.arguments[2])?;
        let usage_address = usize::try_from(frame.arguments[3]).map_err(|_| Error::Fault)?;
        let process = ThreadManager::current_process()?;
        let mut address_space = process.address_space();

        if status_address != 0
            && !address_space.user_range_accessible(status_address, WAIT_STATUS_LENGTH, true)
        {
            return Err(Error::Fault);
        }
        if usage_address != 0
            && !address_space.user_range_accessible(usage_address, RUSAGE_LENGTH, true)
        {
            return Err(Error::Fault);
        }

        loop {
            let status = process.child_wait_status(
                process_id,
                true,
                options.stopped,
                options.continued,
                false,
            )?;
            if let Some(status) = status {
                write_status(&mut address_space, status_address, status)?;
                write_usage(&mut address_space, usage_address, status)?;
                return Ok(i64::from(status.process_id));
            }
            if options.no_hang {
                return Ok(0);
            }

            let (_, blocked_signals) = ThreadManager::current_signal_context()?;
            let pending = ThreadManager::pending_current_signals()?
                .union(process.pending_signals())
                .difference(blocked_signals)
                .remove(SIGCHLD);
            if !pending.is_empty() {
                ThreadManager::take_current_signal_wake();
                return Err(Error::Intr);
            }
            if !ThreadManager::switch_current_for_signal() {
                if ThreadManager::take_current_signal_wake() {
                    continue;
                }
                crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
            }
        }
    }
}

fn parse_options(raw_options: u64) -> Result<WaitOptions, Error> {
    let options = raw_options as u32;
    if options & !VALID_OPTIONS != 0 {
        return Err(Error::Inval);
    }
    Ok(WaitOptions {
        no_hang: options & WNOHANG != 0,
        stopped: options & WUNTRACED != 0,
        continued: options & WCONTINUED != 0,
    })
}

fn write_status(
    address_space: &mut crate::kernel::scheduler::thread::vm::UserAddressSpace,
    address: usize,
    status: ChildExitStatus,
) -> Result<(), Error> {
    if address == 0 {
        return Ok(());
    }
    address_space
        .write(address, &wait_status(status.code, status.status).to_ne_bytes())
        .map_err(|_| Error::Fault)
}

fn write_usage(
    address_space: &mut crate::kernel::scheduler::thread::vm::UserAddressSpace,
    address: usize,
    status: ChildExitStatus,
) -> Result<(), Error> {
    if address == 0 {
        return Ok(());
    }
    let usage = encode_usage(status.resource_usage_snapshot())?;
    address_space
        .write(address, &usage)
        .map_err(|_| Error::Fault)
}

fn wait_status(code: i32, status: i32) -> i32 {
    match code {
        CLD_EXITED => (status & 0xff) << 8,
        CLD_KILLED => status & 0x7f,
        CLD_STOPPED => ((status & 0xff) << 8) | 0x7f,
        CLD_CONTINUED => 0xffff,
        _ => 0,
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/wait4.rs"]
mod tests;