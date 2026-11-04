use core::ffi::{c_int, c_void};
use super::super::errno::ENOSYS;

fn unavailable() -> c_int {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    -1
}

// FIXME: Kernel event readiness sources are not represented by POSIX file descriptors.
#[no_mangle]
pub unsafe extern "C" fn select(
    _descriptor_count: c_int,
    _read_set: *mut c_void,
    _write_set: *mut c_void,
    _error_set: *mut c_void,
    _timeout: *mut c_void,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn pselect(
    _descriptor_count: c_int,
    _read_set: *mut c_void,
    _write_set: *mut c_void,
    _error_set: *mut c_void,
    _timeout: *const c_void,
    _signal_mask: *const c_void,
) -> c_int {
    unavailable()
}