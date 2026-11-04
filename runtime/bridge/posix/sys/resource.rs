use core::ffi::c_int;

use super::super::errno::ENOSYS;

fn set_errno(error: c_int) {
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
}

// FIXME: The kernel runtime has no process resource accounting or limit facility.
#[no_mangle]
pub unsafe extern "C" fn getpriority(_which: c_int, _who: u32) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn getrlimit(_resource: c_int, _limit: *mut core::ffi::c_void) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn getrusage(_who: c_int, _usage: *mut core::ffi::c_void) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn setrlimit(_resource: c_int, _limit: *const core::ffi::c_void) -> c_int {
    set_errno(ENOSYS);
    -1
}