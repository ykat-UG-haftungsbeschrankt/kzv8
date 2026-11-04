use core::ffi::{c_char, c_int};
use super::errno::ENOSYS;

#[repr(C)]
pub struct Passwd {
    name: *mut c_char,
    user_id: u32,
    group_id: u32,
    directory: *mut c_char,
    shell: *mut c_char,
}

// FIXME: The kernel and runtime have no user account database.
fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub unsafe extern "C" fn endpwent() {
    set_errno(ENOSYS);
}

#[no_mangle]
pub unsafe extern "C" fn getpwent() -> *mut Passwd {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getpwnam(_name: *const c_char) -> *mut Passwd {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getpwnam_r(
    _name: *const c_char,
    _buffer: *mut Passwd,
    _scratch: *mut c_char,
    _scratch_size: usize,
    result: *mut *mut Passwd,
) -> c_int {
    if !result.is_null() {
        unsafe { result.write(core::ptr::null_mut()) };
    }
    ENOSYS
}

#[no_mangle]
pub unsafe extern "C" fn getpwuid(_user_id: u32) -> *mut Passwd {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getpwuid_r(
    _user_id: u32,
    _buffer: *mut Passwd,
    _scratch: *mut c_char,
    _scratch_size: usize,
    result: *mut *mut Passwd,
) -> c_int {
    if !result.is_null() {
        unsafe { result.write(core::ptr::null_mut()) };
    }
    ENOSYS
}

#[no_mangle]
pub unsafe extern "C" fn setpwent() {
    set_errno(ENOSYS);
}