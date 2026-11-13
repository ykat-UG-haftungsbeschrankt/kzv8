/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_int, c_void};
use core::mem::{size_of, MaybeUninit};
use core::ptr::read_unaligned;

use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::scheduler::thread::{ThreadAttributes, ThreadManager};

use super::errno::{as_errno, ENOSYS};
use super::sched::{self, SchedParam};

const PTHREAD_CANCEL_ENABLE: c_int = 0;
const PTHREAD_CANCEL_DISABLE: c_int = 1;
const PTHREAD_CANCEL_DEFERRED: c_int = 0;
const PTHREAD_CANCEL_ASYNCHRONOUS: c_int = 1;
const PTHREAD_INHERIT_SCHED: c_int = 0;
const PTHREAD_EXPLICIT_SCHED: c_int = 1;
const SCHED_OTHER: c_int = 0;
const SCHED_FIFO: c_int = 1;
const SCHED_RR: c_int = 2;
const SCHED_SPORADIC: c_int = 3;
const SCHED_PRIORITY_MIN: c_int = 1;
const SCHED_PRIORITY_MAX: c_int = 99;

macro_rules! unsupported_pthread {
    ($name:ident($($argument:ident: $argument_type:ty),* $(,)?)) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name($($argument: $argument_type),*) -> c_int {
            ENOSYS
        }
    };
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_init(attributes: *mut c_void) -> c_int {
    if attributes.is_null() {
        return as_errno(Error::Inval);
    }
    unsafe {
        attributes.cast::<ThreadAttributes>().write(ThreadAttributes {
            stack_size: 0,
            detach_state: 0,
            stack_address: core::ptr::null_mut(),
            inherit_schedule: PTHREAD_INHERIT_SCHED,
            scheduler_policy: SCHED_OTHER,
            scheduler_priority: 0,
            sporadic_low_priority: 0,
            sporadic_period_ns: 0,
            sporadic_budget_ns: 0,
            sporadic_max_replenishments: 0,
        });
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_destroy(attributes: *mut c_void) -> c_int {
    if attributes.is_null() {
        return as_errno(Error::Inval);
    }
    unsafe {
        attributes.cast::<ThreadAttributes>().write(ThreadAttributes {
            stack_size: 0,
            detach_state: 0,
            stack_address: core::ptr::null_mut(),
            inherit_schedule: PTHREAD_INHERIT_SCHED,
            scheduler_policy: SCHED_OTHER,
            scheduler_priority: 0,
            sporadic_low_priority: 0,
            sporadic_period_ns: 0,
            sporadic_budget_ns: 0,
            sporadic_max_replenishments: 0,
        });
    }
    0
}

// FIXME: The kernel has no signal cancellation, priority scheduler, barrier, rwlock, or robust mutex API.
unsupported_pthread!(pthread_atfork(
    _prepare: Option<unsafe extern "C" fn()>,
    _parent: Option<unsafe extern "C" fn()>,
    _child: Option<unsafe extern "C" fn()>,
));
unsupported_pthread!(pthread_attr_getguardsize(_attributes: *const c_void, _guard_size: *mut usize));
#[no_mangle]
pub unsafe extern "C" fn pthread_attr_getinheritsched(
    attributes: *const c_void,
    inherit_schedule: *mut c_int,
) -> c_int {
    if attributes.is_null() || inherit_schedule.is_null() {
        return as_errno(Error::Inval);
    }
    unsafe {
        inherit_schedule.write((*attributes.cast::<ThreadAttributes>()).inherit_schedule)
    };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_getschedparam(
    attributes: *const c_void,
    parameters: *mut c_void,
) -> c_int {
    if attributes.is_null() || parameters.is_null() {
        return as_errno(Error::Inval);
    }
    let attributes = unsafe { &*attributes.cast::<ThreadAttributes>() };
    let scheduling = crate::kernel::process::SchedulingParameters {
        policy: attributes.scheduler_policy,
        priority: attributes.scheduler_priority,
        sporadic: (attributes.scheduler_policy == SCHED_SPORADIC).then_some(
            crate::kernel::process::SporadicParameters {
                low_priority: attributes.sporadic_low_priority,
                replenishment_period_ns: attributes.sporadic_period_ns,
                initial_budget_ns: attributes.sporadic_budget_ns,
                max_replenishments: attributes.sporadic_max_replenishments,
            },
        ),
    };
    unsafe {
        parameters
            .cast::<SchedParam>()
            .write(sched::sched_param_for_configuration(scheduling))
    };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_getschedpolicy(
    attributes: *const c_void,
    policy: *mut c_int,
) -> c_int {
    if attributes.is_null() || policy.is_null() {
        return as_errno(Error::Inval);
    }
    unsafe { policy.write((*attributes.cast::<ThreadAttributes>()).scheduler_policy) };
    0
}
unsupported_pthread!(pthread_attr_getscope(_attributes: *const c_void, _scope: *mut c_int));
unsupported_pthread!(pthread_attr_setguardsize(_attributes: *mut c_void, _guard_size: usize));
#[no_mangle]
pub unsafe extern "C" fn pthread_attr_setinheritsched(
    attributes: *mut c_void,
    inherit_schedule: c_int,
) -> c_int {
    if attributes.is_null() || !matches!(inherit_schedule, PTHREAD_INHERIT_SCHED | PTHREAD_EXPLICIT_SCHED) {
        return as_errno(Error::Inval);
    }
    unsafe { (*attributes.cast::<ThreadAttributes>()).inherit_schedule = inherit_schedule };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_setschedparam(
    attributes: *mut c_void,
    parameters: *const c_void,
) -> c_int {
    if attributes.is_null() || parameters.is_null() {
        return as_errno(Error::Inval);
    }
    let parameter = unsafe { read_unaligned(parameters.cast::<SchedParam>()) };
    let attributes = unsafe { &mut *attributes.cast::<ThreadAttributes>() };
    let scheduling = match sched::scheduling_configuration_from_param(
        attributes.scheduler_policy,
        parameter,
    ) {
        Ok(scheduling) => scheduling,
        Err(error) => return as_errno(error),
    };
    attributes.scheduler_priority = scheduling.priority;
    if let Some(sporadic) = scheduling.sporadic {
        attributes.sporadic_low_priority = sporadic.low_priority;
        attributes.sporadic_period_ns = sporadic.replenishment_period_ns;
        attributes.sporadic_budget_ns = sporadic.initial_budget_ns;
        attributes.sporadic_max_replenishments = sporadic.max_replenishments;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_setschedpolicy(
    attributes: *mut c_void,
    policy: c_int,
) -> c_int {
    if attributes.is_null()
        || !matches!(policy, SCHED_OTHER | SCHED_FIFO | SCHED_RR | SCHED_SPORADIC)
    {
        return as_errno(Error::Inval);
    }
    unsafe { (*attributes.cast::<ThreadAttributes>()).scheduler_policy = policy };
    0
}
unsupported_pthread!(pthread_attr_setscope(_attributes: *mut c_void, _scope: c_int));
unsupported_pthread!(pthread_attr_setstack(_attributes: *mut c_void, _stack_address: *mut c_void, _stack_size: usize));

unsupported_pthread!(pthread_barrier_destroy(_barrier: *mut c_void));
unsupported_pthread!(pthread_barrier_init(_barrier: *mut c_void, _attributes: *const c_void, _count: u32));
unsupported_pthread!(pthread_barrier_wait(_barrier: *mut c_void));
unsupported_pthread!(pthread_barrierattr_destroy(_attributes: *mut c_void));
unsupported_pthread!(pthread_barrierattr_getpshared(_attributes: *const c_void, _process_shared: *mut c_int));
unsupported_pthread!(pthread_barrierattr_init(_attributes: *mut c_void));
unsupported_pthread!(pthread_barrierattr_setpshared(_attributes: *mut c_void, _process_shared: c_int));

fn write_optional_int(
    process: &Process,
    output: *mut c_int,
    value: c_int,
) -> Result<(), Error> {
    if output.is_null() {
        return Ok(());
    }
    let bytes = unsafe {
        core::slice::from_raw_parts((&value as *const c_int).cast::<u8>(), size_of::<c_int>())
    };
    process.address_space().write(output as usize, bytes)
}

#[no_mangle]
pub unsafe extern "C" fn pthread_cancel(thread: usize) -> c_int {
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => return as_errno(error),
    };
    match ThreadManager::request_cancellation(thread, process.pid()) {
        Ok(()) => 0,
        Err(error) => as_errno(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_setcancelstate(
    state: c_int,
    old_state: *mut c_int,
) -> c_int {
    if !matches!(state, PTHREAD_CANCEL_ENABLE | PTHREAD_CANCEL_DISABLE) {
        return as_errno(Error::Inval);
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => return as_errno(error),
    };
    let enabled = state == PTHREAD_CANCEL_ENABLE;
    let previous = match ThreadManager::set_current_cancellation_enabled(enabled) {
        Ok(previous) => previous,
        Err(error) => return as_errno(error),
    };
    let old_state_value = if previous {
        PTHREAD_CANCEL_ENABLE
    } else {
        PTHREAD_CANCEL_DISABLE
    };
    if let Err(error) = write_optional_int(&process, old_state, old_state_value) {
        let _ = ThreadManager::set_current_cancellation_enabled(previous);
        return as_errno(error);
    }
    if enabled {
        ThreadManager::cancel_current_if_requested();
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_setcanceltype(
    cancel_type: c_int,
    old_type: *mut c_int,
) -> c_int {
    if cancel_type == PTHREAD_CANCEL_ASYNCHRONOUS {
        return ENOSYS;
    }
    if cancel_type != PTHREAD_CANCEL_DEFERRED {
        return as_errno(Error::Inval);
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => return as_errno(error),
    };
    match write_optional_int(&process, old_type, PTHREAD_CANCEL_DEFERRED) {
        Ok(()) => 0,
        Err(error) => as_errno(error),
    }
}
#[no_mangle]
pub unsafe extern "C" fn pthread_cond_clockwait(
    condition: *mut c_void,
    mutex: *mut c_void,
    clock_id: c_int,
    timeout: *const c_void,
) -> c_int {
    unsafe {
        crate::kernel::runtime::runtime_libc::pthread::timed_condition_wait(
            condition, mutex, clock_id, timeout,
        )
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_cond_timedwait(
    condition: *mut c_void,
    mutex: *mut c_void,
    timeout: *const c_void,
) -> c_int {
    unsafe {
        crate::kernel::runtime::runtime_libc::pthread::timed_condition_wait(
            condition, mutex, 0, timeout,
        )
    }
}
unsupported_pthread!(pthread_condattr_destroy(_attributes: *mut c_void));
unsupported_pthread!(pthread_condattr_getclock(_attributes: *const c_void, _clock_id: *mut c_int));
unsupported_pthread!(pthread_condattr_getpshared(_attributes: *const c_void, _process_shared: *mut c_int));
unsupported_pthread!(pthread_condattr_init(_attributes: *mut c_void));
unsupported_pthread!(pthread_condattr_setclock(_attributes: *mut c_void, _clock_id: c_int));
unsupported_pthread!(pthread_condattr_setpshared(_attributes: *mut c_void, _process_shared: c_int));

unsupported_pthread!(pthread_getcpuclockid(_thread: usize, _clock_id: *mut c_int));
unsupported_pthread!(pthread_setconcurrency(_concurrency: c_int));
unsupported_pthread!(pthread_getconcurrency());

fn scheduling_process(thread: usize) -> Result<alloc::sync::Arc<Process>, Error> {
    let process = ThreadManager::current_process()?;
    if ThreadManager::thread_belongs_to_process(thread, process.pid()) {
        Ok(process)
    } else {
        Err(Error::Srch)
    }
}

fn read_sched_param(process: &Process, parameters: *const c_void) -> Result<SchedParam, Error> {
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

fn write_sched_outputs(
    process: &Process,
    policy: *mut c_int,
    parameters: *mut c_void,
    policy_value: c_int,
    scheduling: crate::kernel::process::SchedulingParameters,
) -> Result<(), Error> {
    if policy.is_null() || parameters.is_null() {
        return Err(Error::Fault);
    }
    let parameter_value = super::sched::sched_param_for_configuration(scheduling);
    let policy_bytes = unsafe {
        core::slice::from_raw_parts(
            (&policy_value as *const c_int).cast::<u8>(),
            size_of::<c_int>(),
        )
    };
    let parameter_bytes = unsafe {
        core::slice::from_raw_parts(
            (&parameter_value as *const SchedParam).cast::<u8>(),
            size_of::<SchedParam>(),
        )
    };
    let mut address_space = process.address_space();
    address_space.write(policy as usize, policy_bytes)?;
    address_space.write(parameters as usize, parameter_bytes)
}

#[no_mangle]
pub unsafe extern "C" fn pthread_getschedparam(
    thread: usize,
    policy: *mut c_int,
    parameters: *mut c_void,
) -> c_int {
    let process = match scheduling_process(thread) {
        Ok(process) => process,
        Err(error) => return as_errno(error),
    };
    let scheduling =
        match ThreadManager::scheduling_configuration_for_thread(thread) {
            Ok(parameters) => parameters,
            Err(error) => return as_errno(error),
        };
    match write_sched_outputs(&process, policy, parameters, scheduling.policy, scheduling) {
        Ok(()) => 0,
        Err(error) => as_errno(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_setschedparam(
    thread: usize,
    policy: c_int,
    parameters: *const c_void,
) -> c_int {
    let process = match scheduling_process(thread) {
        Ok(process) => process,
        Err(error) => return as_errno(error),
    };
    let parameters = match read_sched_param(&process, parameters) {
        Ok(parameters) => parameters,
        Err(error) => return as_errno(error),
    };
    let scheduling = match super::sched::scheduling_configuration_from_param(policy, parameters) {
        Ok(scheduling) => scheduling,
        Err(error) => return as_errno(error),
    };
    if let Err(error) = super::sched::validate_scheduling_configuration(&process, scheduling) {
        return as_errno(error);
    }
    match ThreadManager::set_scheduling_configuration_for_thread(thread, scheduling) {
        Ok(()) => 0,
        Err(error) => as_errno(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_setschedprio(thread: usize, priority: c_int) -> c_int {
    let process = match scheduling_process(thread) {
        Ok(process) => process,
        Err(error) => return as_errno(error),
    };
    let mut scheduling = match ThreadManager::scheduling_configuration_for_thread(thread) {
        Ok(parameters) => parameters,
        Err(error) => return as_errno(error),
    };
    scheduling.priority = priority;
    if let Err(error) = super::sched::validate_scheduling_configuration(&process, scheduling) {
        return as_errno(error);
    }
    match ThreadManager::set_scheduling_configuration_for_thread(thread, scheduling) {
        Ok(()) => 0,
        Err(error) => as_errno(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_mutex_clocklock(
    mutex: *mut c_void,
    clock_id: c_int,
    timeout: *const c_void,
) -> c_int {
    unsafe {
        crate::kernel::runtime::runtime_libc::pthread::timed_mutex_lock(
            mutex, clock_id, timeout,
        )
    }
}
unsupported_pthread!(pthread_mutex_consistent(_mutex: *mut c_void));
unsupported_pthread!(pthread_mutex_getprioceiling(_mutex: *const c_void, _priority_ceiling: *mut c_int));
unsupported_pthread!(pthread_mutex_setprioceiling(_mutex: *mut c_void, _priority_ceiling: c_int, _old_priority_ceiling: *mut c_int));
#[no_mangle]
pub unsafe extern "C" fn pthread_mutex_timedlock(
    mutex: *mut c_void,
    timeout: *const c_void,
) -> c_int {
    unsafe {
        crate::kernel::runtime::runtime_libc::pthread::timed_mutex_lock(
            mutex, 0, timeout,
        )
    }
}
unsupported_pthread!(pthread_mutexattr_destroy(_attributes: *mut c_void));
unsupported_pthread!(pthread_mutexattr_getprioceiling(_attributes: *const c_void, _priority_ceiling: *mut c_int));
unsupported_pthread!(pthread_mutexattr_getprotocol(_attributes: *const c_void, _protocol: *mut c_int));
unsupported_pthread!(pthread_mutexattr_getpshared(_attributes: *const c_void, _process_shared: *mut c_int));
unsupported_pthread!(pthread_mutexattr_getrobust(_attributes: *const c_void, _robustness: *mut c_int));
unsupported_pthread!(pthread_mutexattr_gettype(_attributes: *const c_void, _mutex_type: *mut c_int));
unsupported_pthread!(pthread_mutexattr_init(_attributes: *mut c_void));
unsupported_pthread!(pthread_mutexattr_setprioceiling(_attributes: *mut c_void, _priority_ceiling: c_int));
unsupported_pthread!(pthread_mutexattr_setprotocol(_attributes: *mut c_void, _protocol: c_int));
unsupported_pthread!(pthread_mutexattr_setpshared(_attributes: *mut c_void, _process_shared: c_int));
unsupported_pthread!(pthread_mutexattr_setrobust(_attributes: *mut c_void, _robustness: c_int));
unsupported_pthread!(pthread_mutexattr_settype(_attributes: *mut c_void, _mutex_type: c_int));

unsupported_pthread!(pthread_rwlock_clockrdlock(_lock: *mut c_void, _clock_id: c_int, _timeout: *const c_void));
unsupported_pthread!(pthread_rwlock_clockwrlock(_lock: *mut c_void, _clock_id: c_int, _timeout: *const c_void));
unsupported_pthread!(pthread_rwlock_destroy(_lock: *mut c_void));
unsupported_pthread!(pthread_rwlock_init(_lock: *mut c_void, _attributes: *const c_void));
unsupported_pthread!(pthread_rwlock_rdlock(_lock: *mut c_void));
unsupported_pthread!(pthread_rwlock_timedrdlock(_lock: *mut c_void, _timeout: *const c_void));
unsupported_pthread!(pthread_rwlock_timedwrlock(_lock: *mut c_void, _timeout: *const c_void));
unsupported_pthread!(pthread_rwlock_tryrdlock(_lock: *mut c_void));
unsupported_pthread!(pthread_rwlock_trywrlock(_lock: *mut c_void));
unsupported_pthread!(pthread_rwlock_unlock(_lock: *mut c_void));
unsupported_pthread!(pthread_rwlock_wrlock(_lock: *mut c_void));
unsupported_pthread!(pthread_rwlockattr_destroy(_attributes: *mut c_void));
unsupported_pthread!(pthread_rwlockattr_getpshared(_attributes: *const c_void, _process_shared: *mut c_int));
unsupported_pthread!(pthread_rwlockattr_init(_attributes: *mut c_void));
unsupported_pthread!(pthread_rwlockattr_setpshared(_attributes: *mut c_void, _process_shared: c_int));

unsupported_pthread!(pthread_spin_destroy(_lock: *mut c_int));
unsupported_pthread!(pthread_spin_init(_lock: *mut c_int, _process_shared: c_int));
unsupported_pthread!(pthread_spin_lock(_lock: *mut c_int));
unsupported_pthread!(pthread_spin_trylock(_lock: *mut c_int));
unsupported_pthread!(pthread_spin_unlock(_lock: *mut c_int));

#[no_mangle]
pub unsafe extern "C" fn pthread_testcancel() {
    ThreadManager::cancel_current_if_requested();
}