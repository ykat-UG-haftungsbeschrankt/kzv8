use core::ffi::{c_int, c_void};
use super::errno::ENOSYS;

// FIXME: The kernel has no asynchronous I/O queue, completion tracking, or cancellation API.
fn unavailable() -> c_int {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn aio_cancel(_file_descriptor: c_int, _control_block: *mut c_void) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn aio_error(_control_block: *const c_void) -> c_int {
    ENOSYS
}

#[no_mangle]
pub unsafe extern "C" fn aio_fsync(_operation: c_int, _control_block: *mut c_void) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn aio_read(_control_block: *mut c_void) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn aio_return(_control_block: *mut c_void) -> isize {
    unavailable() as isize
}

#[no_mangle]
pub unsafe extern "C" fn aio_suspend(
    _list: *const *const c_void,
    _count: c_int,
    _timeout: *const c_void,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn aio_write(_control_block: *mut c_void) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn lio_listio(
    _mode: c_int,
    _list: *const *mut c_void,
    _count: c_int,
    _event: *mut c_void,
) -> c_int {
    unavailable()
}