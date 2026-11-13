/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::c_int;
use core::mem::{size_of, MaybeUninit};

use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::scheduler::thread::ThreadManager;

use super::errno::as_errno;

const SCHED_OTHER: c_int = 0;
const SCHED_FIFO: c_int = 1;
const SCHED_RR: c_int = 2;
const SCHED_SPORADIC: c_int = 3;
const REALTIME_PRIORITY_MIN: c_int = 1;
const REALTIME_PRIORITY_MAX: c_int = 99;
const ROUND_ROBIN_INTERVAL_NS: i64 = 10_000_000;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SchedParam {
    priority: c_int,
}

#[repr(C)]
struct Timespec {
    seconds: i64,
    nanoseconds: i64,
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

fn process_for_id(process_id: i32) -> Result<alloc::sync::Arc<Process>, Error> {
    let process = ThreadManager::current_process()?;
    if process_id == 0 || process_id == process.pid() {
        Ok(process)
    } else {
        Err(Error::Srch)
    }
}

fn priority_range(policy: c_int) -> Result<(c_int, c_int), Error> {
    match policy {
        SCHED_OTHER => Ok((0, 0)),
        SCHED_FIFO | SCHED_RR => Ok((REALTIME_PRIORITY_MIN, REALTIME_PRIORITY_MAX)),
        SCHED_SPORADIC => Err(Error::NoSys),
        _ => Err(Error::Inval),
    }
}

fn read_parameters(
    process: &Process,
    parameters: *const SchedParam,
) -> Result<SchedParam, Error> {
    if parameters.is_null() {
        return Err(Error::Fault);
    }
    let mut value = MaybeUninit::<SchedParam>::uninit();
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), size_of::<SchedParam>())
    };
    process.address_space().read(parameters as usize, bytes)?;
    Ok(unsafe { value.assume_init() })
}

fn write_parameters(
    process: &Process,
    parameters: *mut SchedParam,
    value: SchedParam,
) -> Result<(), Error> {
    if parameters.is_null() {
        return Err(Error::Fault);
    }
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&value as *const SchedParam).cast::<u8>(),
            size_of::<SchedParam>(),
        )
    };
    process.address_space().write(parameters as usize, bytes)
}

fn set_realtime_permission(process: &Process, policy: c_int) -> Result<(), Error> {
    if matches!(policy, SCHED_FIFO | SCHED_RR) && !process.effective_user_is_root() {
        Err(Error::Perm)
    } else {
        Ok(())
    }
}

pub(crate) fn validate_scheduling_parameters(
    process: &Process,
    policy: c_int,
    priority: c_int,
) -> Result<(), Error> {
    let (minimum, maximum) = priority_range(policy)?;
    if !(minimum..=maximum).contains(&priority) {
        return Err(Error::Inval);
    }
    set_realtime_permission(process, policy)
}

#[no_mangle]
pub extern "C" fn sched_get_priority_max(policy: c_int) -> c_int {
    match priority_range(policy) {
        Ok((_, maximum)) => maximum,
        Err(error) => {
            set_errno(as_errno(error));
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn sched_get_priority_min(policy: c_int) -> c_int {
    match priority_range(policy) {
        Ok((minimum, _)) => minimum,
        Err(error) => {
            set_errno(as_errno(error));
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn sched_getparam(
    process_id: i32,
    parameters: *mut SchedParam,
) -> c_int {
    let process = match process_for_id(process_id) {
        Ok(process) => process,
        Err(error) => return fail(error),
    };
    let (_, priority) = process.scheduler_parameters();
    match write_parameters(&process, parameters, SchedParam { priority }) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub extern "C" fn sched_getscheduler(process_id: i32) -> c_int {
    match process_for_id(process_id) {
        Ok(process) => process.scheduler_parameters().0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn sched_rr_get_interval(
    process_id: i32,
    interval: *mut core::ffi::c_void,
) -> c_int {
    let process = match process_for_id(process_id) {
        Ok(process) => process,
        Err(error) => return fail(error),
    };
    if interval.is_null() {
        return fail(Error::Fault);
    }
    let value = Timespec {
        seconds: 0,
        nanoseconds: ROUND_ROBIN_INTERVAL_NS,
    };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&value as *const Timespec).cast::<u8>(),
            size_of::<Timespec>(),
        )
    };
    match process.address_space().write(interval as usize, bytes) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn sched_setparam(
    process_id: i32,
    parameters: *const SchedParam,
) -> c_int {
    let process = match process_for_id(process_id) {
        Ok(process) => process,
        Err(error) => return fail(error),
    };
    let parameters = match read_parameters(&process, parameters) {
        Ok(parameters) => parameters,
        Err(error) => return fail(error),
    };
    let (policy, _) = process.scheduler_parameters();
    let (minimum, maximum) = match priority_range(policy) {
        Ok(range) => range,
        Err(error) => return fail(error),
    };
    if !(minimum..=maximum).contains(&parameters.priority) {
        return fail(Error::Inval);
    }
    if let Err(error) = set_realtime_permission(&process, policy) {
        return fail(error);
    }
    process.set_scheduler_parameters(policy, parameters.priority);
    0
}

#[no_mangle]
pub unsafe extern "C" fn sched_setscheduler(
    process_id: i32,
    policy: c_int,
    parameters: *const SchedParam,
) -> c_int {
    let process = match process_for_id(process_id) {
        Ok(process) => process,
        Err(error) => return fail(error),
    };
    let parameters = match read_parameters(&process, parameters) {
        Ok(parameters) => parameters,
        Err(error) => return fail(error),
    };
    let (minimum, maximum) = match priority_range(policy) {
        Ok(range) => range,
        Err(error) => return fail(error),
    };
    if !(minimum..=maximum).contains(&parameters.priority) {
        return fail(Error::Inval);
    }
    if let Err(error) = set_realtime_permission(&process, policy) {
        return fail(error);
    }
    process.set_scheduler_parameters(policy, parameters.priority);
    0
}

fn fail(error: Error) -> c_int {
    set_errno(as_errno(error));
    -1
}