use core::ffi::{c_char, c_int, c_void};
use core::sync::atomic::{AtomicU32, Ordering};
use super::errno::{EFAULT, ENOSYS};

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub unsafe extern "C" fn sem_getvalue(semaphore: *mut u32, output: *mut c_int) -> c_int {
    if semaphore.is_null() || output.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let value = unsafe { &*semaphore.cast::<AtomicU32>() }.load(Ordering::Acquire);
    unsafe { output.write(value as c_int) };
    0
}

// FIXME: The kernel has no named-semaphore store or timed semaphore wait queue.
#[no_mangle]
pub unsafe extern "C" fn sem_close(_semaphore: *mut c_void) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn sem_timedwait(
    _semaphore: *mut c_void,
    _timeout: *const c_void,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn sem_clockwait(
    _semaphore: *mut c_void,
    _clock_id: c_int,
    _timeout: *const c_void,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn sem_unlink(_name: *const c_char) -> c_int {
    set_errno(ENOSYS);
    -1
}