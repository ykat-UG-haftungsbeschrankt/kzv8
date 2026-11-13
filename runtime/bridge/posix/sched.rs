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
use crate::kernel::process::{Process, SchedulingParameters, SporadicParameters};
use crate::kernel::scheduler::thread::ThreadManager;

use super::errno::as_errno;

const SCHED_OTHER: c_int = 0;
const SCHED_FIFO: c_int = 1;
const SCHED_RR: c_int = 2;
const SCHED_SPORADIC: c_int = 3;
const REALTIME_PRIORITY_MIN: c_int = 1;
const REALTIME_PRIORITY_MAX: c_int = 99;
const ROUND_ROBIN_INTERVAL_NS: i64 = 10_000_000;
const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SchedParam {
    pub(crate) priority: c_int,
    pub(crate) low_priority: c_int,
    pub(crate) replenishment_period: Timespec,
    pub(crate) initial_budget: Timespec,
    pub(crate) max_replenishments: c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct Timespec {
    pub(crate) seconds: i64,
    pub(crate) nanoseconds: i64,
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

fn process_for_id(process_id: i32) -> Result<alloc::sync::Arc<Process>, Error> {
    if process_id == 0 {
        ThreadManager::current_process()
    } else {
        Process::lookup(process_id)
    }
}

fn priority_range(policy: c_int) -> Result<(c_int, c_int), Error> {
    match policy {
        SCHED_OTHER => Ok((0, 0)),
        SCHED_FIFO | SCHED_RR | SCHED_SPORADIC => {
            Ok((REALTIME_PRIORITY_MIN, REALTIME_PRIORITY_MAX))
        }
        _ => Err(Error::Inval),
    }
}

fn timespec_to_ns(value: Timespec) -> Result<u64, Error> {
    if value.seconds < 0 || !(0..1_000_000_000).contains(&value.nanoseconds) {
        return Err(Error::Inval);
    }
    u64::try_from(value.seconds)
        .ok()
        .and_then(|seconds| seconds.checked_mul(NANOSECONDS_PER_SECOND))
        .and_then(|seconds| seconds.checked_add(value.nanoseconds as u64))
        .ok_or(Error::Inval)
}

fn ns_to_timespec(value: u64) -> Timespec {
    Timespec {
        seconds: (value / NANOSECONDS_PER_SECOND) as i64,
        nanoseconds: (value % NANOSECONDS_PER_SECOND) as i64,
    }
}

fn validate_configuration_shape(scheduling: SchedulingParameters) -> Result<(), Error> {
    let (minimum, maximum) = priority_range(scheduling.policy)?;
    if !(minimum..=maximum).contains(&scheduling.priority) {
        return Err(Error::Inval);
    }
    if scheduling.policy == SCHED_SPORADIC {
        let parameters = scheduling.sporadic.ok_or(Error::Inval)?;
        if !(REALTIME_PRIORITY_MIN..=REALTIME_PRIORITY_MAX).contains(&parameters.low_priority)
            || parameters.low_priority >= scheduling.priority
            || parameters.initial_budget_ns == 0
            || parameters.replenishment_period_ns == 0
            || parameters.initial_budget_ns > parameters.replenishment_period_ns
            || parameters.max_replenishments == 0
            || parameters.max_replenishments
                > crate::kernel::scheduler::thread::MAX_SPORADIC_REPLENISHMENTS
        {
            return Err(Error::Inval);
        }
    } else if scheduling.sporadic.is_some() {
        return Err(Error::Inval);
    }
    Ok(())
}

pub(crate) fn scheduling_configuration_from_param(
    policy: c_int,
    parameters: SchedParam,
) -> Result<SchedulingParameters, Error> {
    let sporadic = if policy == SCHED_SPORADIC {
        Some(SporadicParameters {
            low_priority: parameters.low_priority,
            replenishment_period_ns: timespec_to_ns(parameters.replenishment_period)?,
            initial_budget_ns: timespec_to_ns(parameters.initial_budget)?,
            max_replenishments: u32::try_from(parameters.max_replenishments)
                .map_err(|_| Error::Inval)?,
        })
    } else {
        None
    };
    let scheduling = SchedulingParameters {
        policy,
        priority: parameters.priority,
        sporadic,
    };
    validate_configuration_shape(scheduling)?;
    Ok(scheduling)
}

pub(crate) fn sched_param_for_configuration(scheduling: SchedulingParameters) -> SchedParam {
    let sporadic = scheduling.sporadic;
    SchedParam {
        priority: scheduling.priority,
        low_priority: sporadic.map_or(0, |parameters| parameters.low_priority),
        replenishment_period: sporadic
            .map_or(Timespec::default(), |parameters| ns_to_timespec(parameters.replenishment_period_ns)),
        initial_budget: sporadic
            .map_or(Timespec::default(), |parameters| ns_to_timespec(parameters.initial_budget_ns)),
        max_replenishments: sporadic.map_or(0, |parameters| parameters.max_replenishments as c_int),
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

fn check_scheduling_permission(caller: &Process, target: &Process, policy: c_int) -> Result<(), Error> {
    if !target.may_change_scheduling(caller) {
        return Err(Error::Perm);
    }
    if matches!(policy, SCHED_FIFO | SCHED_RR | SCHED_SPORADIC)
        && !caller.effective_user_is_root()
    {
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
    validate_scheduling_configuration(
        process,
        SchedulingParameters {
            policy,
            priority,
            sporadic: None,
        },
    )
}

pub(crate) fn validate_scheduling_configuration(
    process: &Process,
    scheduling: SchedulingParameters,
) -> Result<(), Error> {
    validate_configuration_shape(scheduling)?;
    check_scheduling_permission(process, process, scheduling.policy)
}

fn caller_can_query(target: &Process) -> Result<(), Error> {
    let caller = ThreadManager::current_process()?;
    if target.may_change_scheduling(&caller) {
        Ok(())
    } else {
        Err(Error::Perm)
    }
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
    if let Err(error) = caller_can_query(&process) {
        return fail(error);
    }
    let scheduling = process.scheduling_configuration();
    match write_parameters(&process, parameters, sched_param_for_configuration(scheduling)) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub extern "C" fn sched_getscheduler(process_id: i32) -> c_int {
    match process_for_id(process_id) {
        Ok(process) => match caller_can_query(&process) {
            Ok(()) => process.scheduler_parameters().0,
            Err(error) => fail(error),
        },
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
    if let Err(error) = caller_can_query(&process) {
        return fail(error);
    }
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
    let caller = match ThreadManager::current_process() {
        Ok(caller) => caller,
        Err(error) => return fail(error),
    };
    let parameters = match read_parameters(&process, parameters) {
        Ok(parameters) => parameters,
        Err(error) => return fail(error),
    };
    let policy = process.scheduler_parameters().0;
    let scheduling = match scheduling_configuration_from_param(policy, parameters) {
        Ok(scheduling) => scheduling,
        Err(error) => return fail(error),
    };
    if let Err(error) = validate_scheduling_configuration(&caller, scheduling) {
        return fail(error);
    }
    ThreadManager::set_process_scheduling_configuration(&process, scheduling);
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
    let caller = match ThreadManager::current_process() {
        Ok(caller) => caller,
        Err(error) => return fail(error),
    };
    let parameters = match read_parameters(&process, parameters) {
        Ok(parameters) => parameters,
        Err(error) => return fail(error),
    };
    let scheduling = match scheduling_configuration_from_param(policy, parameters) {
        Ok(scheduling) => scheduling,
        Err(error) => return fail(error),
    };
    if let Err(error) = validate_scheduling_configuration(&caller, scheduling) {
        return fail(error);
    }
    ThreadManager::set_process_scheduling_configuration(&process, scheduling);
    0
}

fn fail(error: Error) -> c_int {
    set_errno(as_errno(error));
    -1
}