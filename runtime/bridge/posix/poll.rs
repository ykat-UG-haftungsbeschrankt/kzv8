use core::ffi::{c_int, c_void};
use super::errno::ENOSYS;

fn unavailable() -> c_int {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    -1
}

// FIXME: Kernel event readiness sources have no POSIX file-descriptor mapping.
#[no_mangle]
pub unsafe extern "C" fn poll(
    _descriptors: *mut c_void,
    _descriptor_count: u64,
    _timeout: c_int,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn ppoll(
    _descriptors: *mut c_void,
    _descriptor_count: u64,
    _timeout: *const c_void,
    _signal_mask: *const c_void,
) -> c_int {
    unavailable()
}