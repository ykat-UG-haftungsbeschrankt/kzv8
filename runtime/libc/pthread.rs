/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::c_void;
use core::sync::atomic::Ordering;

use super::errno::{EAGAIN, EBUSY, EDEADLK, EINVAL, ENOMEM, ENOSYS, EPERM, ESRCH};
use crate::kernel::arch::traits::{InterruptArchitecture as _, ThreadArchitecture as _};
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::{
    PthreadCondition, PthreadMutex, StartRoutine, ThreadAttributes, ThreadManager, ThreadStack,
    ThreadState,
};

const JOINABLE: i32 = 0;
const DETACHED: i32 = 1;

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
    ThreadManager::exit_current(result)
}

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
    unsafe {
        attributes
            .cast::<ThreadAttributes>()
            .write(ThreadAttributes {
                stack_size,
                detach_state: if detached { DETACHED } else { JOINABLE },
                stack_address,
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

    let (stack_size, detach_state) = if attributes.is_null() {
        (0, JOINABLE)
    } else {
        let attributes = unsafe { &*attributes.cast::<ThreadAttributes>() };
        (attributes.stack_size, attributes.detach_state)
    };
    let stack_size = if stack_size == 0 {
        ThreadStack::DEFAULT_SIZE
    } else {
        stack_size
    };
    if stack_size < ThreadStack::MIN_SIZE || !matches!(detach_state, JOINABLE | DETACHED) {
        return EINVAL;
    }

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
    unsafe { thread_id.write(id) };
    0
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
        if !ThreadManager::switch_current(ThreadState::Ready) {
            let lock_status = unsafe { pthread_mutex_lock(mutex) };
            return if lock_status == 0 {
                EDEADLK
            } else {
                lock_status
            };
        }
    }
    unsafe { pthread_mutex_lock(mutex) }
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
