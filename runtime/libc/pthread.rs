use core::ffi::c_void;

#[no_mangle]
pub extern "C" fn pthread_self() -> usize {
    crate::kernel_thread::current_thread_id()
}

#[no_mangle]
pub extern "C" fn pthread_equal(first: usize, second: usize) -> i32 {
    (first == second) as i32
}

#[no_mangle]
pub extern "C" fn sched_yield() -> i32 {
    crate::kernel_thread::yield_current()
}

#[no_mangle]
pub extern "C" fn pthread_yield() -> i32 {
    sched_yield()
}

#[no_mangle]
pub extern "C" fn pthread_exit(result: *mut c_void) -> ! {
    crate::kernel_thread::exit_current(result)
}

macro_rules! export_pthread {
    ($name:ident($($argument:ident: $argument_type:ty),* $(,)?) -> $result:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name($($argument: $argument_type),*) -> $result {
            unsafe { crate::kernel_thread::$name($($argument),*) }
        }
    };
}

export_pthread!(
    pthread_key_create(key: *mut u32, destructor: Option<unsafe extern "C" fn(*mut c_void)>) -> i32
);
export_pthread!(pthread_key_delete(key: u32) -> i32);
export_pthread!(pthread_getspecific(key: u32) -> *mut c_void);
export_pthread!(pthread_setspecific(key: u32, value: *const c_void) -> i32);
export_pthread!(pthread_once(once: *mut u32, initializer: Option<unsafe extern "C" fn()>) -> i32);
export_pthread!(
    pthread_getschedparam(thread_id: usize, policy: *mut i32, parameters: *mut c_void) -> i32
);
export_pthread!(pthread_attr_init(attributes: *mut c_void) -> i32);
export_pthread!(pthread_attr_destroy(attributes: *mut c_void) -> i32);
export_pthread!(pthread_attr_setdetachstate(attributes: *mut c_void, detach_state: i32) -> i32);
export_pthread!(
    pthread_attr_getdetachstate(attributes: *const c_void, detach_state: *mut i32) -> i32
);
export_pthread!(pthread_attr_setstacksize(attributes: *mut c_void, stack_size: usize) -> i32);
export_pthread!(
    pthread_attr_getstacksize(attributes: *const c_void, stack_size: *mut usize) -> i32
);
export_pthread!(
    pthread_attr_getstack(
        attributes: *const c_void,
        stack_address: *mut *mut c_void,
        stack_size: *mut usize,
    ) -> i32
);
export_pthread!(pthread_getattr_np(thread_id: usize, attributes: *mut c_void) -> i32);
export_pthread!(pthread_kill(thread_id: usize, signal_number: i32) -> i32);
export_pthread!(
    pthread_create(
        thread_id: *mut usize,
        attributes: *const c_void,
        start_routine: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
        argument: *mut c_void,
    ) -> i32
);
export_pthread!(pthread_join(thread_id: usize, result: *mut *mut c_void) -> i32);
export_pthread!(pthread_detach(thread_id: usize) -> i32);
export_pthread!(pthread_mutex_init(mutex: *mut c_void, attributes: *const c_void) -> i32);
export_pthread!(pthread_mutex_destroy(mutex: *mut c_void) -> i32);
export_pthread!(pthread_mutex_lock(mutex: *mut c_void) -> i32);
export_pthread!(pthread_mutex_trylock(mutex: *mut c_void) -> i32);
export_pthread!(pthread_mutex_unlock(mutex: *mut c_void) -> i32);
export_pthread!(pthread_cond_init(condition: *mut c_void, attributes: *const c_void) -> i32);
export_pthread!(pthread_cond_destroy(condition: *mut c_void) -> i32);
export_pthread!(pthread_cond_wait(condition: *mut c_void, mutex: *mut c_void) -> i32);
export_pthread!(pthread_cond_signal(condition: *mut c_void) -> i32);
export_pthread!(pthread_cond_broadcast(condition: *mut c_void) -> i32);
