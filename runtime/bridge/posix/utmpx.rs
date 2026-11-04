use core::ffi::{c_int, c_void};

use super::errno::ENOSYS;

fn set_errno(error: c_int) {
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
}

fn unavailable_entry() -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

// FIXME: The kernel runtime has no user-accounting database.
#[no_mangle]
pub extern "C" fn getutxent() -> *mut c_void {
    unavailable_entry()
}

#[no_mangle]
pub unsafe extern "C" fn getutxid(_entry: *const c_void) -> *mut c_void {
    unavailable_entry()
}

#[no_mangle]
pub unsafe extern "C" fn getutxline(_entry: *const c_void) -> *mut c_void {
    unavailable_entry()
}

#[no_mangle]
pub unsafe extern "C" fn pututxline(_entry: *const c_void) -> *mut c_void {
    unavailable_entry()
}

#[no_mangle]
pub extern "C" fn setutxent() {
    set_errno(ENOSYS);
}

#[no_mangle]
pub extern "C" fn endutxent() {
    set_errno(ENOSYS);
}