use core::ffi::{c_char, c_int, c_void};
use super::super::errno::ENOSYS;

fn unavailable() -> c_int {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    -1
}

// FIXME: The filesystem exposes no total/free block or inode capacity information.
#[no_mangle]
pub unsafe extern "C" fn fstatvfs(_file_descriptor: c_int, _status: *mut c_void) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn statvfs(_path: *const c_char, _status: *mut c_void) -> c_int {
    unavailable()
}