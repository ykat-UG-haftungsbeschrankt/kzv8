/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_int, c_void};
use core::ptr::read_unaligned;
use core::sync::atomic::Ordering;

use super::errno::{EAGAIN, EBUSY, EDEADLK, EINVAL, ENOMEM, ENOSYS, EPERM, ESRCH, ETIMEDOUT};
#[allow(unused_imports)]
use crate::kernel::arch::traits::{
    CpuArchitecture as _, InterruptArchitecture as _, ThreadArchitecture as _, TimerArchitecture as _,
};
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::{
    PthreadCondition, PthreadMutex, StartRoutine, ThreadAttributes, ThreadManager, ThreadStack,
    ThreadState,
};

const JOINABLE: i32 = 0;
const DETACHED: i32 = 1;
const INHERIT_SCHED: i32 = 0;
const EXPLICIT_SCHED: i32 = 1;
const SCHED_OTHER: i32 = 0;
const SCHED_FIFO: i32 = 1;
const SCHED_RR: i32 = 2;
const SCHED_PRIORITY_MIN: i32 = 1;
const SCHED_PRIORITY_MAX: i32 = 99;
const CLOCK_REALTIME: c_int = 0;
const CLOCK_MONOTONIC: c_int = 1;
const CLOCK_MONOTONIC_RAW: c_int = 4;

#[repr(C)]
#[derive(Clone, Copy)]
struct CTimespec {
    seconds: i64,
    nanoseconds: i64,
}

unsafe extern "C" {
    fn clock_gettime(clock_id: c_int, output: *mut CTimespec) -> c_int;
}

fn monotonic_now_ns() -> u64 {
    crate::kernel::arch::TimerArchitecture::now_ns()
        .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns)
}

unsafe fn absolute_deadline_ns(clock_id: c_int, timeout: *const c_void) -> Result<u64, i32> {
    if !matches!(clock_id, CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW)
        || timeout.is_null()
    {
        return Err(EINVAL);
    }
    let timeout = unsafe { read_unaligned(timeout.cast::<CTimespec>()) };
    if timeout.seconds < 0 || !(0..1_000_000_000).contains(&timeout.nanoseconds) {
        return Err(EINVAL);
    }
    let mut now = CTimespec {
        seconds: 0,
        nanoseconds: 0,
    };
    if unsafe { clock_gettime(clock_id, &mut now) } != 0 {
        return Err(unsafe { crate::kernel::scheduler::thread::__errno_location().read() });
    }
    let target_ns = i128::from(timeout.seconds) * 1_000_000_000
        + i128::from(timeout.nanoseconds);
    let now_ns = i128::from(now.seconds) * 1_000_000_000 + i128::from(now.nanoseconds);
    let delay_ns = u64::try_from((target_ns - now_ns).max(0)).map_err(|_| EINVAL)?;
    monotonic_now_ns().checked_add(delay_ns).ok_or(EINVAL)
}

fn mutex_lock_until(mutex: *mut c_void, deadline_ns: u64) -> i32 {
    if mutex.is_null() {
        return EINVAL;
    }
    let owner = unsafe { &(*mutex.cast::<PthreadMutex>()).owner };
    let current_id = ThreadManager::current_thread_id();
    if current_id == 0 {
        return EAGAIN;
    }
    loop {
        ThreadManager::cancel_current_if_requested();
        match owner.compare_exchange(0, current_id, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => return 0,
            Err(existing) if existing == current_id => return EDEADLK,
            Err(_) if monotonic_now_ns() >= deadline_ns => return ETIMEDOUT,
            Err(_) => {
                if !ThreadManager::switch_current(ThreadState::Ready) {
                    crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
                }
            }
        }
    }
}

pub(crate) unsafe fn timed_mutex_lock(
    mutex: *mut c_void,
    clock_id: c_int,
    timeout: *const c_void,
) -> i32 {
    let deadline_ns = match unsafe { absolute_deadline_ns(clock_id, timeout) } {
        Ok(deadline_ns) => deadline_ns,
        Err(error) => return error,
    };
    mutex_lock_until(mutex, deadline_ns)
}

pub(crate) unsafe fn timed_condition_wait(
    condition: *mut c_void,
    mutex: *mut c_void,
    clock_id: c_int,
    timeout: *const c_void,
) -> i32 {
    if condition.is_null() || mutex.is_null() {
        return EINVAL;
    }
    let deadline_ns = match unsafe { absolute_deadline_ns(clock_id, timeout) } {
        Ok(deadline_ns) => deadline_ns,
        Err(error) => return error,
    };
    let generation = unsafe {
        (*condition.cast::<PthreadCondition>())
            .generation
            .load(Ordering::Acquire)
    };
    let unlock_status = unsafe { pthread_mutex_unlock(mutex) };
    if unlock_status != 0 {
        return unlock_status;
    }
    let condition_generation = unsafe { &(*condition.cast::<PthreadCondition>()).generation };
    loop {
        if ThreadManager::cancellation_requested_current() {
            let lock_status = unsafe { pthread_mutex_lock(mutex) };
            if lock_status != 0 {
                return lock_status;
            }
            ThreadManager::cancel_current_if_requested();
        }
        if condition_generation.load(Ordering::Acquire) != generation {
            return unsafe { pthread_mutex_lock(mutex) };
        }
        if monotonic_now_ns() >= deadline_ns {
            let lock_status = unsafe { pthread_mutex_lock(mutex) };
            return if lock_status == 0 {
                ETIMEDOUT
            } else {
                lock_status
            };
        }
        if !ThreadManager::switch_current(ThreadState::Ready) {
            crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
        }
    }
}

#[no_mangle]
pub extern "C" fn pthread_self() -> usize {
    ThreadManager::current_thread_id()
}

#[no_mangle]
pub extern "C" fn pthread_equal(first: usize, second: usize) -> i32 {
    (first == second) as i32
}

#[no_mangle]
pub extern "C" fn sched_yield() -> i32 {
    let _ = ThreadManager::switch_current(ThreadState::Ready);
    0
}

#[no_mangle]
pub extern "C" fn pthread_yield() -> i32 {
    sched_yield()
}

#[no_mangle]
pub extern "C" fn pthread_exit(result: *mut c_void) -> ! {
    ThreadManager::run_cleanup_handlers_current();
    ThreadManager::run_tls_destructors_current();
    ThreadManager::exit_current(result)
}

#[no_mangle]
pub unsafe extern "C" fn __pthread_cleanup_push(
    routine: Option<unsafe extern "C" fn(*mut c_void)>,
    argument: *mut c_void,
) -> i32 {
    let Some(routine) = routine else {
        return EINVAL;
    };
    ThreadManager::push_cleanup_handler(routine, argument).map_or_else(
        |error| match error {
            Error::NoMem => ENOMEM,
            _ => EINVAL,
        },
        |_| 0,
    )
}

#[no_mangle]
pub unsafe extern "C" fn __pthread_cleanup_pop(execute: c_int) -> i32 {
    ThreadManager::pop_cleanup_handler(execute != 0).map_or_else(
        |error| match error {
            Error::Inval => EINVAL,
            _ => EINVAL,
        },
        |_| 0,
    )
}

#[allow(unused_macros)]
macro_rules! export_pthread {
    ($name:ident($($argument:ident: $argument_type:ty),* $(,)?) -> $result:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name($($argument: $argument_type),*) -> $result {
            unsafe { crate::kernel::scheduler::thread::$name($($argument),*) }
        }
    };
}

#[no_mangle]
pub unsafe extern "C" fn pthread_key_create(
    key: *mut u32,
    destructor: Option<unsafe extern "C" fn(*mut c_void)>,
) -> i32 {
    if key.is_null() {
        return EINVAL;
    }
    let Some(created_key) = ThreadManager::create_tls_key(destructor) else {
        return EAGAIN;
    };
    unsafe { key.write(created_key) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_key_delete(key: u32) -> i32 {
    if ThreadManager::delete_tls_key(key) {
        0
    } else {
        EINVAL
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_getspecific(key: u32) -> *mut c_void {
    ThreadManager::get_specific(key).unwrap_or(core::ptr::null_mut())
}

#[no_mangle]
pub unsafe extern "C" fn pthread_setspecific(key: u32, value: *const c_void) -> i32 {
    if ThreadManager::set_specific(key, value) {
        0
    } else {
        EINVAL
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_once(
    once: *mut u32,
    initializer: Option<unsafe extern "C" fn()>,
) -> i32 {
    if once.is_null() || initializer.is_none() {
        return EINVAL;
    }
    let state = unsafe { &*once.cast::<core::sync::atomic::AtomicU32>() };
    loop {
        match state.load(Ordering::Acquire) {
            2 => return 0,
            0 => {
                if state
                    .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    unsafe { initializer.unwrap()() };
                    state.store(2, Ordering::Release);
                    return 0;
                }
            }
            _ => {
                if !ThreadManager::switch_current(ThreadState::Ready) {
                    core::hint::spin_loop();
                }
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_getschedparam(
    thread_id: usize,
    policy: *mut i32,
    parameters: *mut c_void,
) -> i32 {
    if policy.is_null() || parameters.is_null() {
        return EINVAL;
    }
    if !ThreadManager::contains_thread_by_id(thread_id) {
        return ESRCH;
    }
    unsafe {
        policy.write(0);
        parameters.cast::<i32>().write(0);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_init(attributes: *mut c_void) -> i32 {
    if attributes.is_null() {
        return EINVAL;
    }
    unsafe {
        attributes
            .cast::<ThreadAttributes>()
            .write(ThreadAttributes {
                stack_size: 0,
                detach_state: JOINABLE,
                stack_address: core::ptr::null_mut(),
                inherit_schedule: INHERIT_SCHED,
                scheduler_policy: SCHED_OTHER,
                scheduler_priority: 0,
            });
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_destroy(attributes: *mut c_void) -> i32 {
    if attributes.is_null() {
        return EINVAL;
    }
    unsafe {
        attributes
            .cast::<ThreadAttributes>()
            .write(ThreadAttributes {
                stack_size: 0,
                detach_state: JOINABLE,
                stack_address: core::ptr::null_mut(),
                inherit_schedule: INHERIT_SCHED,
                scheduler_policy: SCHED_OTHER,
                scheduler_priority: 0,
            });
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_setdetachstate(
    attributes: *mut c_void,
    detach_state: i32,
) -> i32 {
    if attributes.is_null() || !matches!(detach_state, JOINABLE | DETACHED) {
        return EINVAL;
    }
    unsafe { (*attributes.cast::<ThreadAttributes>()).detach_state = detach_state };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_getdetachstate(
    attributes: *const c_void,
    detach_state: *mut i32,
) -> i32 {
    if attributes.is_null() || detach_state.is_null() {
        return EINVAL;
    }
    unsafe { detach_state.write((*attributes.cast::<ThreadAttributes>()).detach_state) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_setinheritsched(
    attributes: *mut c_void,
    inherit_schedule: i32,
) -> i32 {
    if attributes.is_null() || !matches!(inherit_schedule, INHERIT_SCHED | EXPLICIT_SCHED) {
        return EINVAL;
    }
    unsafe { (*attributes.cast::<ThreadAttributes>()).inherit_schedule = inherit_schedule };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_getinheritsched(
    attributes: *const c_void,
    inherit_schedule: *mut i32,
) -> i32 {
    if attributes.is_null() || inherit_schedule.is_null() {
        return EINVAL;
    }
    unsafe {
        inherit_schedule.write((*attributes.cast::<ThreadAttributes>()).inherit_schedule)
    };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_setschedpolicy(
    attributes: *mut c_void,
    policy: i32,
) -> i32 {
    if attributes.is_null() || !matches!(policy, SCHED_OTHER | SCHED_FIFO | SCHED_RR) {
        return EINVAL;
    }
    unsafe { (*attributes.cast::<ThreadAttributes>()).scheduler_policy = policy };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_getschedpolicy(
    attributes: *const c_void,
    policy: *mut i32,
) -> i32 {
    if attributes.is_null() || policy.is_null() {
        return EINVAL;
    }
    unsafe { policy.write((*attributes.cast::<ThreadAttributes>()).scheduler_policy) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_setschedparam(
    attributes: *mut c_void,
    parameters: *const c_void,
) -> i32 {
    if attributes.is_null() || parameters.is_null() {
        return EINVAL;
    }
    let priority = unsafe { read_unaligned(parameters.cast::<i32>()) };
    let attributes = unsafe { &mut *attributes.cast::<ThreadAttributes>() };
    if !valid_scheduling_parameters(attributes.scheduler_policy, priority) {
        return EINVAL;
    }
    attributes.scheduler_priority = priority;
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_getschedparam(
    attributes: *const c_void,
    parameters: *mut c_void,
) -> i32 {
    if attributes.is_null() || parameters.is_null() {
        return EINVAL;
    }
    let priority = unsafe { (*attributes.cast::<ThreadAttributes>()).scheduler_priority };
    unsafe { parameters.cast::<i32>().write(priority) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_setstacksize(attributes: *mut c_void, stack_size: usize) -> i32 {
    if attributes.is_null() || stack_size < ThreadStack::MIN_SIZE {
        return EINVAL;
    }
    unsafe { (*attributes.cast::<ThreadAttributes>()).stack_size = stack_size };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_getstacksize(
    attributes: *const c_void,
    stack_size: *mut usize,
) -> i32 {
    if attributes.is_null() || stack_size.is_null() {
        return EINVAL;
    }
    let stored_size = unsafe { (*attributes.cast::<ThreadAttributes>()).stack_size };
    unsafe {
        stack_size.write(if stored_size == 0 {
            ThreadStack::DEFAULT_SIZE
        } else {
            stored_size
        })
    };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_getstack(
    attributes: *const c_void,
    stack_address: *mut *mut c_void,
    stack_size: *mut usize,
) -> i32 {
    if attributes.is_null() || stack_address.is_null() || stack_size.is_null() {
        return EINVAL;
    }
    let attributes = unsafe { &*attributes.cast::<ThreadAttributes>() };
    unsafe {
        stack_address.write(attributes.stack_address);
        stack_size.write(if attributes.stack_size == 0 {
            ThreadStack::DEFAULT_SIZE
        } else {
            attributes.stack_size
        });
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_getattr_np(thread_id: usize, attributes: *mut c_void) -> i32 {
    if attributes.is_null() {
        return EINVAL;
    }
    let Some((stack_address, stack_size, detached)) =
        ThreadManager::thread_attributes_by_id(thread_id)
    else {
        return ESRCH;
    };
    let (scheduler_policy, scheduler_priority) =
        ThreadManager::scheduling_parameters_for_thread(thread_id).unwrap_or((SCHED_OTHER, 0));
    unsafe {
        attributes
            .cast::<ThreadAttributes>()
            .write(ThreadAttributes {
                stack_size,
                detach_state: if detached { DETACHED } else { JOINABLE },
                stack_address,
                inherit_schedule: EXPLICIT_SCHED,
                scheduler_policy,
                scheduler_priority,
            });
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_kill(thread_id: usize, signal_number: i32) -> i32 {
    if !ThreadManager::contains_thread_by_id(thread_id) {
        return ESRCH;
    }
    if signal_number == 0 {
        0
    } else {
        ENOSYS
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_create(
    thread_id: *mut usize,
    attributes: *const c_void,
    start_routine: Option<StartRoutine>,
    argument: *mut c_void,
) -> i32 {
    if thread_id.is_null() || start_routine.is_none() {
        return EINVAL;
    }

    let (stack_size, detach_state, inherit_schedule, scheduler_policy, scheduler_priority) =
        if attributes.is_null() {
        (0, JOINABLE, INHERIT_SCHED, SCHED_OTHER, 0)
    } else {
        let attributes = unsafe { &*attributes.cast::<ThreadAttributes>() };
        (
            attributes.stack_size,
            attributes.detach_state,
            attributes.inherit_schedule,
            attributes.scheduler_policy,
            attributes.scheduler_priority,
        )
    };
    let stack_size = if stack_size == 0 {
        ThreadStack::DEFAULT_SIZE
    } else {
        stack_size
    };
    if stack_size < ThreadStack::MIN_SIZE
        || !matches!(detach_state, JOINABLE | DETACHED)
        || !matches!(inherit_schedule, INHERIT_SCHED | EXPLICIT_SCHED)
    {
        return EINVAL;
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(_) => return EAGAIN,
    };
    let (stack_limit, _) = match process.resource_limit(3) {
        Ok(limit) => limit,
        Err(_) => return EINVAL,
    };
    if u64::try_from(stack_size).map_or(true, |size| size > stack_limit) {
        return EAGAIN;
    }

    let scheduling_parameters = if inherit_schedule == INHERIT_SCHED {
        match ThreadManager::scheduling_parameters_for_thread(ThreadManager::current_thread_id()) {
            Ok(parameters) => parameters,
            Err(_) => return EAGAIN,
        }
    } else {
        if !valid_scheduling_parameters(scheduler_policy, scheduler_priority) {
            return EINVAL;
        }
        if matches!(scheduler_policy, SCHED_FIFO | SCHED_RR)
            && !ThreadManager::current_process()
                .is_ok_and(|process| process.effective_user_is_root())
        {
            return EPERM;
        }
        (scheduler_policy, scheduler_priority)
    };

    let start_routine = start_routine.unwrap();
    let id = match ThreadManager::create_thread(
        stack_size,
        start_routine,
        argument,
        detach_state == DETACHED,
    ) {
        Ok(id) => id,
        Err(Error::Again) => return EAGAIN,
        Err(Error::NoMem) => return ENOMEM,
        Err(Error::Inval) => return EINVAL,
        Err(_) => return EINVAL,
    };
    if ThreadManager::set_scheduling_parameters_for_thread(
        id,
        scheduling_parameters.0,
        scheduling_parameters.1,
    )
    .is_err()
    {
        return EAGAIN;
    }
    unsafe { thread_id.write(id) };
    0
}

fn valid_scheduling_parameters(policy: i32, priority: i32) -> bool {
    match policy {
        SCHED_OTHER => priority == 0,
        SCHED_FIFO | SCHED_RR => (SCHED_PRIORITY_MIN..=SCHED_PRIORITY_MAX).contains(&priority),
        _ => false,
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_join(thread_id: usize, result: *mut *mut c_void) -> i32 {
    match ThreadManager::begin_join_by_id(thread_id) {
        Ok(Some(value)) => {
            if !result.is_null() {
                unsafe { result.write(value as *mut c_void) };
            }
            return 0;
        }
        Ok(None) => {}
        Err(error) => return join_error_status(error),
    }

    if !ThreadManager::switch_current(ThreadState::Blocked) {
        ThreadManager::cancel_join_by_id(thread_id);
        return EDEADLK;
    }
    if ThreadManager::cancellation_requested_current() {
        ThreadManager::cancel_join_by_id(thread_id);
        ThreadManager::cancel_current_if_requested();
    }

    let value = match ThreadManager::finish_join_by_id(thread_id) {
        Ok(value) => value,
        Err(error) => return join_error_status(error),
    };
    if !result.is_null() {
        unsafe { result.write(value as *mut c_void) };
    }
    0
}

fn join_error_status(error: Error) -> i32 {
    match error {
        Error::Srch => ESRCH,
        Error::Deadlk => EDEADLK,
        Error::Inval => EINVAL,
        _ => EINVAL,
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_detach(thread_id: usize) -> i32 {
    match ThreadManager::detach_by_id(thread_id) {
        Ok(()) => 0,
        Err(Error::Srch) => ESRCH,
        Err(_) => EINVAL,
    }
}

#[allow(dead_code)]
pub(crate) fn yield_current() -> i32 {
    let _ = ThreadManager::switch_current(ThreadState::Ready);
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_mutex_init(mutex: *mut c_void, attributes: *const c_void) -> i32 {
    if mutex.is_null() || !attributes.is_null() {
        return EINVAL;
    }
    unsafe {
        (*mutex.cast::<PthreadMutex>())
            .owner
            .store(0, Ordering::Relaxed)
    };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_mutex_destroy(mutex: *mut c_void) -> i32 {
    if mutex.is_null() {
        return EINVAL;
    }
    let owner = unsafe {
        (*mutex.cast::<PthreadMutex>())
            .owner
            .load(Ordering::Relaxed)
    };
    if owner != 0 {
        EBUSY
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_mutex_lock(mutex: *mut c_void) -> i32 {
    if mutex.is_null() {
        return EINVAL;
    }
    let owner = unsafe { &(*mutex.cast::<PthreadMutex>()).owner };
    let current_id = ThreadManager::current_thread_id();
    if current_id == 0 {
        return EAGAIN;
    }
    loop {
        ThreadManager::cancel_current_if_requested();
        match owner.compare_exchange(0, current_id, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => return 0,
            Err(existing) if existing == current_id => return EDEADLK,
            Err(_) if !ThreadManager::switch_current(ThreadState::Ready) => return EDEADLK,
            Err(_) => {}
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_mutex_trylock(mutex: *mut c_void) -> i32 {
    if mutex.is_null() {
        return EINVAL;
    }
    let owner = unsafe { &(*mutex.cast::<PthreadMutex>()).owner };
    let current_id = ThreadManager::current_thread_id();
    if current_id == 0 {
        return EAGAIN;
    }
    match owner.compare_exchange(0, current_id, Ordering::Acquire, Ordering::Relaxed) {
        Ok(_) => 0,
        Err(_) => EBUSY,
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_mutex_unlock(mutex: *mut c_void) -> i32 {
    if mutex.is_null() {
        return EINVAL;
    }
    let owner = unsafe { &(*mutex.cast::<PthreadMutex>()).owner };
    let current_id = ThreadManager::current_thread_id();
    if current_id == 0 {
        return EPERM;
    }
    match owner.compare_exchange(current_id, 0, Ordering::Release, Ordering::Relaxed) {
        Ok(_) => 0,
        Err(_) => EPERM,
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_cond_init(condition: *mut c_void, attributes: *const c_void) -> i32 {
    if condition.is_null() || !attributes.is_null() {
        return EINVAL;
    }
    unsafe {
        (*condition.cast::<PthreadCondition>())
            .generation
            .store(0, Ordering::Relaxed)
    };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_cond_destroy(condition: *mut c_void) -> i32 {
    if condition.is_null() {
        EINVAL
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_cond_wait(condition: *mut c_void, mutex: *mut c_void) -> i32 {
    if condition.is_null() || mutex.is_null() {
        return EINVAL;
    }
    let generation = unsafe {
        (*condition.cast::<PthreadCondition>())
            .generation
            .load(Ordering::Acquire)
    };
    let unlock_status = unsafe { pthread_mutex_unlock(mutex) };
    if unlock_status != 0 {
        return unlock_status;
    }

    let condition_generation = unsafe { &(*condition.cast::<PthreadCondition>()).generation };
    while condition_generation.load(Ordering::Acquire) == generation {
        if ThreadManager::cancellation_requested_current() {
            let lock_status = unsafe { pthread_mutex_lock(mutex) };
            if lock_status != 0 {
                return lock_status;
            }
            ThreadManager::cancel_current_if_requested();
        }
        if !ThreadManager::switch_current(ThreadState::Ready) {
            let lock_status = unsafe { pthread_mutex_lock(mutex) };
            return if lock_status == 0 {
                EDEADLK
            } else {
                lock_status
            };
        }
    }
    let lock_status = unsafe { pthread_mutex_lock(mutex) };
    if lock_status == 0 {
        ThreadManager::cancel_current_if_requested();
    }
    lock_status
}

#[no_mangle]
pub unsafe extern "C" fn pthread_cond_signal(condition: *mut c_void) -> i32 {
    if condition.is_null() {
        return EINVAL;
    }
    unsafe {
        (*condition.cast::<PthreadCondition>())
            .generation
            .fetch_add(1, Ordering::Release);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_cond_broadcast(condition: *mut c_void) -> i32 {
    unsafe { pthread_cond_signal(condition) }
}
