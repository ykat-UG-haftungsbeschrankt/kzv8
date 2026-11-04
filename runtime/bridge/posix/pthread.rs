use core::ffi::{c_int, c_void};
use super::errno::ENOSYS;

macro_rules! unsupported_pthread {
    ($name:ident($($argument:ident: $argument_type:ty),* $(,)?)) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name($($argument: $argument_type),*) -> c_int {
            ENOSYS
        }
    };
}

// FIXME: The kernel has no signal cancellation, priority scheduler, barrier, rwlock, or robust mutex API.
unsupported_pthread!(pthread_atfork(
    _prepare: Option<unsafe extern "C" fn()>,
    _parent: Option<unsafe extern "C" fn()>,
    _child: Option<unsafe extern "C" fn()>,
));
unsupported_pthread!(pthread_attr_getguardsize(_attributes: *const c_void, _guard_size: *mut usize));
unsupported_pthread!(pthread_attr_getinheritsched(_attributes: *const c_void, _inherit_schedule: *mut c_int));
unsupported_pthread!(pthread_attr_getschedparam(_attributes: *const c_void, _parameters: *mut c_void));
unsupported_pthread!(pthread_attr_getschedpolicy(_attributes: *const c_void, _policy: *mut c_int));
unsupported_pthread!(pthread_attr_getscope(_attributes: *const c_void, _scope: *mut c_int));
unsupported_pthread!(pthread_attr_setguardsize(_attributes: *mut c_void, _guard_size: usize));
unsupported_pthread!(pthread_attr_setinheritsched(_attributes: *mut c_void, _inherit_schedule: c_int));
unsupported_pthread!(pthread_attr_setschedparam(_attributes: *mut c_void, _parameters: *const c_void));
unsupported_pthread!(pthread_attr_setschedpolicy(_attributes: *mut c_void, _policy: c_int));
unsupported_pthread!(pthread_attr_setscope(_attributes: *mut c_void, _scope: c_int));
unsupported_pthread!(pthread_attr_setstack(_attributes: *mut c_void, _stack_address: *mut c_void, _stack_size: usize));

unsupported_pthread!(pthread_barrier_destroy(_barrier: *mut c_void));
unsupported_pthread!(pthread_barrier_init(_barrier: *mut c_void, _attributes: *const c_void, _count: u32));
unsupported_pthread!(pthread_barrier_wait(_barrier: *mut c_void));
unsupported_pthread!(pthread_barrierattr_destroy(_attributes: *mut c_void));
unsupported_pthread!(pthread_barrierattr_getpshared(_attributes: *const c_void, _process_shared: *mut c_int));
unsupported_pthread!(pthread_barrierattr_init(_attributes: *mut c_void));
unsupported_pthread!(pthread_barrierattr_setpshared(_attributes: *mut c_void, _process_shared: c_int));

unsupported_pthread!(pthread_cancel(_thread: usize));
unsupported_pthread!(pthread_cond_clockwait(_condition: *mut c_void, _mutex: *mut c_void, _clock_id: c_int, _timeout: *const c_void));
unsupported_pthread!(pthread_cond_timedwait(_condition: *mut c_void, _mutex: *mut c_void, _timeout: *const c_void));
unsupported_pthread!(pthread_condattr_destroy(_attributes: *mut c_void));
unsupported_pthread!(pthread_condattr_getclock(_attributes: *const c_void, _clock_id: *mut c_int));
unsupported_pthread!(pthread_condattr_getpshared(_attributes: *const c_void, _process_shared: *mut c_int));
unsupported_pthread!(pthread_condattr_init(_attributes: *mut c_void));
unsupported_pthread!(pthread_condattr_setclock(_attributes: *mut c_void, _clock_id: c_int));
unsupported_pthread!(pthread_condattr_setpshared(_attributes: *mut c_void, _process_shared: c_int));

unsupported_pthread!(pthread_getcpuclockid(_thread: usize, _clock_id: *mut c_int));
unsupported_pthread!(pthread_setschedparam(_thread: usize, _policy: c_int, _parameters: *const c_void));
unsupported_pthread!(pthread_setschedprio(_thread: usize, _priority: c_int));
unsupported_pthread!(pthread_setconcurrency(_concurrency: c_int));
unsupported_pthread!(pthread_getconcurrency());

unsupported_pthread!(pthread_mutex_clocklock(_mutex: *mut c_void, _clock_id: c_int, _timeout: *const c_void));
unsupported_pthread!(pthread_mutex_consistent(_mutex: *mut c_void));
unsupported_pthread!(pthread_mutex_getprioceiling(_mutex: *const c_void, _priority_ceiling: *mut c_int));
unsupported_pthread!(pthread_mutex_setprioceiling(_mutex: *mut c_void, _priority_ceiling: c_int, _old_priority_ceiling: *mut c_int));
unsupported_pthread!(pthread_mutex_timedlock(_mutex: *mut c_void, _timeout: *const c_void));
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
pub unsafe extern "C" fn pthread_testcancel() {}