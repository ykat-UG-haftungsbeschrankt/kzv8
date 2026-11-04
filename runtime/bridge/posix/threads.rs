use alloc::boxed::Box;
use core::ffi::{c_int, c_void};

const THRD_SUCCESS: c_int = 0;
const THRD_BUSY: c_int = 1;
const THRD_ERROR: c_int = 2;
const THRD_NOMEM: c_int = 3;
const THRD_TIMEDOUT: c_int = 4;
const MTX_RECURSIVE: c_int = 2;

type StartRoutine = unsafe extern "C" fn(*mut c_void) -> c_int;

struct StartContext {
    routine: StartRoutine,
    argument: *mut c_void,
}

unsafe extern "C" {
    fn pthread_create(
        thread: *mut usize,
        attributes: *const c_void,
        routine: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
        argument: *mut c_void,
    ) -> c_int;
    fn pthread_self() -> usize;
    fn pthread_equal(left: usize, right: usize) -> c_int;
    fn pthread_detach(thread: usize) -> c_int;
    fn pthread_join(thread: usize, result: *mut *mut c_void) -> c_int;
    fn pthread_exit(result: *mut c_void) -> !;
    fn pthread_once(once: *mut u32, function: Option<unsafe extern "C" fn()>) -> c_int;
    fn pthread_mutex_init(mutex: *mut c_void, attributes: *const c_void) -> c_int;
    fn pthread_mutex_destroy(mutex: *mut c_void) -> c_int;
    fn pthread_mutex_lock(mutex: *mut c_void) -> c_int;
    fn pthread_mutex_trylock(mutex: *mut c_void) -> c_int;
    fn pthread_mutex_unlock(mutex: *mut c_void) -> c_int;
    fn pthread_mutex_timedlock(mutex: *mut c_void, timeout: *const c_void) -> c_int;
    fn pthread_cond_init(condition: *mut c_void, attributes: *const c_void) -> c_int;
    fn pthread_cond_destroy(condition: *mut c_void) -> c_int;
    fn pthread_cond_wait(condition: *mut c_void, mutex: *mut c_void) -> c_int;
    fn pthread_cond_signal(condition: *mut c_void) -> c_int;
    fn pthread_cond_broadcast(condition: *mut c_void) -> c_int;
    fn pthread_cond_timedwait(
        condition: *mut c_void,
        mutex: *mut c_void,
        timeout: *const c_void,
    ) -> c_int;
    fn pthread_key_create(
        key: *mut u32,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int;
    fn pthread_key_delete(key: u32) -> c_int;
    fn pthread_getspecific(key: u32) -> *mut c_void;
    fn pthread_setspecific(key: u32, value: *const c_void) -> c_int;
    fn sched_yield() -> c_int;
    fn nanosleep(request: *const c_void, remaining: *mut c_void) -> c_int;
}

fn status(error: c_int) -> c_int {
    match error {
        0 => THRD_SUCCESS,
        11 | 12 => THRD_NOMEM,
        16 | 35 => THRD_BUSY,
        110 => THRD_TIMEDOUT,
        _ => THRD_ERROR,
    }
}

unsafe extern "C" fn start_trampoline(context: *mut c_void) -> *mut c_void {
    let context = unsafe { Box::from_raw(context.cast::<StartContext>()) };
    let result = unsafe { (context.routine)(context.argument) };
    result as isize as *mut c_void
}

#[repr(C)]
struct Timespec {
    seconds: i64,
    nanoseconds: i64,
}

#[no_mangle]
pub unsafe extern "C" fn thrd_create(
    thread: *mut usize,
    routine: Option<StartRoutine>,
    argument: *mut c_void,
) -> c_int {
    if thread.is_null() || routine.is_none() {
        return THRD_ERROR;
    }
    let context = Box::new(StartContext {
        routine: routine.unwrap(),
        argument,
    });
    let context = Box::into_raw(context).cast::<c_void>();
    let result = unsafe { pthread_create(thread, core::ptr::null(), Some(start_trampoline), context) };
    if result != 0 {
        unsafe { drop(Box::from_raw(context.cast::<StartContext>())) };
    }
    status(result)
}

#[no_mangle]
pub extern "C" fn thrd_current() -> usize {
    unsafe { pthread_self() }
}

#[no_mangle]
pub extern "C" fn thrd_detach(thread: usize) -> c_int {
    status(unsafe { pthread_detach(thread) })
}

#[no_mangle]
pub extern "C" fn thrd_equal(left: usize, right: usize) -> c_int {
    unsafe { pthread_equal(left, right) }
}

#[no_mangle]
pub extern "C" fn thrd_exit(result: c_int) -> ! {
    unsafe { pthread_exit(result as isize as *mut c_void) }
}

#[no_mangle]
pub unsafe extern "C" fn thrd_join(thread: usize, result: *mut c_int) -> c_int {
    let mut thread_result = core::ptr::null_mut();
    let join_result = unsafe { pthread_join(thread, &mut thread_result) };
    if join_result == 0 && !result.is_null() {
        unsafe { result.write(thread_result as isize as c_int) };
    }
    status(join_result)
}

#[no_mangle]
pub unsafe extern "C" fn thrd_sleep(duration: *const Timespec, remaining: *mut Timespec) -> c_int {
    match unsafe { nanosleep(duration.cast(), remaining.cast()) } {
        0 => 0,
        _ => -1,
    }
}

#[no_mangle]
pub extern "C" fn thrd_yield() {
    let _ = unsafe { sched_yield() };
}

#[no_mangle]
pub unsafe extern "C" fn call_once(flag: *mut u32, function: Option<unsafe extern "C" fn()>) {
    let _ = unsafe { pthread_once(flag, function) };
}

#[no_mangle]
pub unsafe extern "C" fn mtx_init(mutex: *mut c_void, kind: c_int) -> c_int {
    if kind & MTX_RECURSIVE != 0 {
        return THRD_ERROR;
    }
    status(unsafe { pthread_mutex_init(mutex, core::ptr::null()) })
}

#[no_mangle]
pub unsafe extern "C" fn mtx_lock(mutex: *mut c_void) -> c_int {
    status(unsafe { pthread_mutex_lock(mutex) })
}

#[no_mangle]
pub unsafe extern "C" fn mtx_timedlock(mutex: *mut c_void, timeout: *const Timespec) -> c_int {
    status(unsafe { pthread_mutex_timedlock(mutex, timeout.cast()) })
}

#[no_mangle]
pub unsafe extern "C" fn mtx_trylock(mutex: *mut c_void) -> c_int {
    status(unsafe { pthread_mutex_trylock(mutex) })
}

#[no_mangle]
pub unsafe extern "C" fn mtx_unlock(mutex: *mut c_void) -> c_int {
    status(unsafe { pthread_mutex_unlock(mutex) })
}

#[no_mangle]
pub unsafe extern "C" fn mtx_destroy(mutex: *mut c_void) {
    let _ = unsafe { pthread_mutex_destroy(mutex) };
}

#[no_mangle]
pub unsafe extern "C" fn cnd_init(condition: *mut c_void) -> c_int {
    status(unsafe { pthread_cond_init(condition, core::ptr::null()) })
}

#[no_mangle]
pub unsafe extern "C" fn cnd_broadcast(condition: *mut c_void) -> c_int {
    status(unsafe { pthread_cond_broadcast(condition) })
}

#[no_mangle]
pub unsafe extern "C" fn cnd_destroy(condition: *mut c_void) {
    let _ = unsafe { pthread_cond_destroy(condition) };
}

#[no_mangle]
pub unsafe extern "C" fn cnd_signal(condition: *mut c_void) -> c_int {
    status(unsafe { pthread_cond_signal(condition) })
}

#[no_mangle]
pub unsafe extern "C" fn cnd_timedwait(
    condition: *mut c_void,
    mutex: *mut c_void,
    timeout: *const Timespec,
) -> c_int {
    status(unsafe { pthread_cond_timedwait(condition, mutex, timeout.cast()) })
}

#[no_mangle]
pub unsafe extern "C" fn cnd_wait(condition: *mut c_void, mutex: *mut c_void) -> c_int {
    status(unsafe { pthread_cond_wait(condition, mutex) })
}

#[no_mangle]
pub unsafe extern "C" fn tss_create(
    key: *mut u32,
    destructor: Option<unsafe extern "C" fn(*mut c_void)>,
) -> c_int {
    status(unsafe { pthread_key_create(key, destructor) })
}

#[no_mangle]
pub extern "C" fn tss_delete(key: u32) {
    let _ = unsafe { pthread_key_delete(key) };
}

#[no_mangle]
pub extern "C" fn tss_get(key: u32) -> *mut c_void {
    unsafe { pthread_getspecific(key) }
}

#[no_mangle]
pub unsafe extern "C" fn tss_set(key: u32, value: *mut c_void) -> c_int {
    status(unsafe { pthread_setspecific(key, value) })
}