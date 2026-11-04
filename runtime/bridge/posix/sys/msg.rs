use core::ffi::{c_int, c_void};
use super::super::errno::ENOSYS;

fn unavailable() -> c_int {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    -1
}

// FIXME: The kernel has no XSI message-queue objects or IPC namespace.
#[no_mangle]
pub unsafe extern "C" fn msgctl(
    _queue_id: c_int,
    _command: c_int,
    _buffer: *mut c_void,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn msgget(_key: i32, _flags: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn msgrcv(
    _queue_id: c_int,
    _message: *mut c_void,
    _message_size: usize,
    _message_type: i64,
    _flags: c_int,
) -> isize {
    unavailable() as isize
}

#[no_mangle]
pub unsafe extern "C" fn msgsnd(
    _queue_id: c_int,
    _message: *const c_void,
    _message_size: usize,
    _flags: c_int,
) -> c_int {
    unavailable()
}