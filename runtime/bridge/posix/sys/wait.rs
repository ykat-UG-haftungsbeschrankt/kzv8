use core::ffi::{c_int, c_void};
use super::super::errno::ENOSYS;

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

// FIXME: The kernel has no child-process lifecycle or waitable process state.
#[no_mangle]
pub unsafe extern "C" fn wait(_status: *mut c_int) -> i32 {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn waitid(
    _type: c_int,
    _identifier: u32,
    _information: *mut c_void,
    _options: c_int,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn waitpid(
    _process_id: i32,
    _status: *mut c_int,
    _options: c_int,
) -> i32 {
    set_errno(ENOSYS);
    -1
}