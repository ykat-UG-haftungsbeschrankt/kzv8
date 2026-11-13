/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_int, c_long, c_void};
use core::mem::{size_of, MaybeUninit};

use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::scheduler::thread::ThreadManager;

use super::super::errno::as_errno;

#[repr(C)]
#[derive(Clone, Copy)]
struct RLimit {
    current: u64,
    maximum: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Timeval {
    seconds: c_long,
    microseconds: c_long,
}

#[repr(C)]
struct RUsage {
    user_time: Timeval,
    system_time: Timeval,
    counters: [c_long; 14],
}

const PRIO_PROCESS: c_int = 0;
const PRIO_PGRP: c_int = 1;
const PRIO_USER: c_int = 2;
const RUSAGE_SELF: c_int = 0;
const RUSAGE_CHILDREN: c_int = -1;

fn processes_for_priority(
    which: c_int,
    who: u32,
) -> Result<(alloc::sync::Arc<Process>, alloc::vec::Vec<alloc::sync::Arc<Process>>), Error> {
    let caller = ThreadManager::current_process()?;
    let mut processes = match which {
        PRIO_PROCESS => {
            let process = if who == 0 {
                alloc::sync::Arc::clone(&caller)
            } else {
                Process::lookup(i32::try_from(who).map_err(|_| Error::Srch)?)?
            };
            alloc::vec![process]
        }
        PRIO_PGRP => {
            let process_group = if who == 0 {
                caller.process_group_id()
            } else {
                i32::try_from(who).map_err(|_| Error::Inval)?
            };
            let mut processes = Process::registered_processes()?;
            processes.retain(|process| process.process_group_id() == process_group);
            processes
        }
        PRIO_USER => {
            let user = if who == 0 { caller.user_ids()[0] } else { who };
            let mut processes = Process::registered_processes()?;
            processes.retain(|process| process.user_ids()[0] == user);
            processes
        }
        _ => return Err(Error::Inval),
    };
    if processes.is_empty() {
        return Err(Error::Srch);
    }
    let selected_count = processes.len();
    processes.retain(|process| process.may_inspect_process(&caller));
    if processes.len() != selected_count {
        return Err(Error::Perm);
    }
    if processes.is_empty() {
        return Err(Error::Perm);
    }
    Ok((caller, processes))
}

fn set_errno(error: c_int) {
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
}

#[no_mangle]
pub unsafe extern "C" fn getpriority(which: c_int, who: u32) -> c_int {
    match processes_for_priority(which, who) {
        Ok((_, processes)) => processes
            .iter()
            .map(|process| process.nice_priority())
            .min()
            .unwrap_or(0),
        Err(error) => {
            set_errno(as_errno(error));
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn getrlimit(_resource: c_int, _limit: *mut core::ffi::c_void) -> c_int {
    if _limit.is_null() {
        return fail(Error::Fault);
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => return fail(error),
    };
    let (current, maximum) = match process.resource_limit(_resource) {
        Ok(limit) => limit,
        Err(error) => return fail(error),
    };
    let value = RLimit { current, maximum };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&value as *const RLimit).cast::<u8>(),
            size_of::<RLimit>(),
        )
    };
    match process.address_space().write(_limit as usize, bytes) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn getrusage(_who: c_int, _usage: *mut core::ffi::c_void) -> c_int {
    if _usage.is_null() {
        return fail(Error::Fault);
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => return fail(error),
    };
    let cpu_time_ns = match _who {
        RUSAGE_SELF => ThreadManager::current_process_cpu_time_ns(),
        RUSAGE_CHILDREN => process.child_cpu_time_ns(),
        _ => return fail(Error::Inval),
    };
    let seconds = match c_long::try_from(cpu_time_ns / 1_000_000_000) {
        Ok(seconds) => seconds,
        Err(_) => return fail(Error::Overflow),
    };
    let microseconds = ((cpu_time_ns % 1_000_000_000) / 1000) as c_long;
    let value = RUsage {
        user_time: Timeval {
            seconds,
            microseconds,
        },
        system_time: Timeval {
            seconds: 0,
            microseconds: 0,
        },
        counters: [0; 14],
    };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&value as *const RUsage).cast::<u8>(),
            size_of::<RUsage>(),
        )
    };
    match process.address_space().write(_usage as usize, bytes) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn setrlimit(_resource: c_int, _limit: *const c_void) -> c_int {
    if _limit.is_null() {
        return fail(Error::Fault);
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => return fail(error),
    };
    let mut value = MaybeUninit::<RLimit>::uninit();
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), size_of::<RLimit>())
    };
    if let Err(error) = process.address_space().read(_limit as usize, bytes) {
        return fail(error);
    }
    let value = unsafe { value.assume_init() };
    match process.set_resource_limit(_resource, value.current, value.maximum) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn setpriority(which: c_int, who: u32, priority: c_int) -> c_int {
    let (caller, processes) = match processes_for_priority(which, who) {
        Ok(targets) => targets,
        Err(error) => return fail(error),
    };
    if !caller.effective_user_is_root()
        && processes
            .iter()
            .any(|process| priority < process.nice_priority())
    {
        return fail(Error::Perm);
    }
    for process in &processes {
        if let Err(error) = process.set_nice_priority(&caller, priority) {
            return fail(error);
        }
    }
    0
}

fn fail(error: Error) -> c_int {
    set_errno(as_errno(error));
    -1
}