use core::ffi::{c_char, c_int, c_void};
use super::errno::ENOSYS;

fn unavailable() -> c_int {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    -1
}

// FIXME: The kernel has no message-queue objects, descriptors, or notification backend.
#[no_mangle]
pub unsafe extern "C" fn mq_close(_descriptor: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn mq_getattr(_descriptor: c_int, _attributes: *mut c_void) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn mq_notify(_descriptor: c_int, _event: *const c_void) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn mq_receive(
    _descriptor: c_int,
    _message: *mut c_char,
    _message_size: usize,
    _priority: *mut u32,
) -> isize {
    unavailable() as isize
}

#[no_mangle]
pub unsafe extern "C" fn mq_send(
    _descriptor: c_int,
    _message: *const c_char,
    _message_size: usize,
    _priority: u32,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn mq_setattr(
    _descriptor: c_int,
    _attributes: *const c_void,
    _old_attributes: *mut c_void,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn mq_timedreceive(
    _descriptor: c_int,
    _message: *mut c_char,
    _message_size: usize,
    _priority: *mut u32,
    _timeout: *const c_void,
) -> isize {
    unavailable() as isize
}

#[no_mangle]
pub unsafe extern "C" fn mq_timedsend(
    _descriptor: c_int,
    _message: *const c_char,
    _message_size: usize,
    _priority: u32,
    _timeout: *const c_void,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn mq_unlink(_name: *const c_char) -> c_int {
    unavailable()
}