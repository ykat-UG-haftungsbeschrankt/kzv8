use core::ffi::{c_int, c_void};
use super::errno::ENOSYS;

// FIXME: The kernel has no device-control dispatch interface for open descriptors.
#[no_mangle]
pub unsafe extern "C" fn posix_devctl(
    _file_descriptor: c_int,
    _command: c_int,
    _control_data: *mut c_void,
    _control_size: usize,
    _device_information: *mut c_int,
) -> c_int {
    ENOSYS
}