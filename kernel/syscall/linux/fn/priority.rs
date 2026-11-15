/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::scheduler::thread::ThreadManager;

const PRIO_PROCESS: i32 = 0;
const PRIO_PGRP: i32 = 1;
const PRIO_USER: i32 = 2;

pub(crate) struct Getpriority;
pub(crate) struct Setpriority;

fn select_targets(
    caller: &Arc<Process>,
    which: i32,
    who: u32,
) -> Result<Vec<Arc<Process>>, Error> {
    let processes = match which {
        PRIO_PROCESS => {
            let process = if who == 0 {
                Arc::clone(caller)
            } else {
                Process::lookup(i32::try_from(who).map_err(|_| Error::Srch)?)?
            };
            vec![process]
        }
        PRIO_PGRP => {
            let process_group = if who == 0 {
                caller.process_group_id()
            } else {
                i32::try_from(who).map_err(|_| Error::Inval)?
            };
            let mut selected = Process::registered_processes()?;
            selected.retain(|process| process.process_group_id() == process_group);
            selected
        }
        PRIO_USER => {
            let user = if who == 0 { caller.user_ids()[0] } else { who };
            let mut selected = Process::registered_processes()?;
            selected.retain(|process| process.user_ids()[0] == user);
            selected
        }
        _ => return Err(Error::Inval),
    };
    if processes.is_empty() {
        return Err(Error::Srch);
    }
    if processes
        .iter()
        .any(|process| !process.may_inspect_process(caller))
    {
        return Err(Error::Perm);
    }
    Ok(processes)
}

fn linux_priority(nice: i32) -> i64 {
    i64::from(20 - nice)
}

impl Getpriority {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Getpriority;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let caller = ThreadManager::current_process()?;
        let processes = select_targets(&caller, frame.arguments[0] as i32, frame.arguments[1] as u32)?;
        let nice = processes
            .iter()
            .map(|process| process.nice_priority())
            .min()
            .ok_or(Error::Srch)?;
        Ok(linux_priority(nice))
    }
}

impl Setpriority {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Setpriority;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let caller = ThreadManager::current_process()?;
        let processes = select_targets(&caller, frame.arguments[0] as i32, frame.arguments[1] as u32)?;
        let nice = frame.arguments[2] as i32;
        for process in processes {
            process.set_nice_priority(&caller, nice)?;
        }
        Ok(0)
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/priority.rs"]
mod tests;