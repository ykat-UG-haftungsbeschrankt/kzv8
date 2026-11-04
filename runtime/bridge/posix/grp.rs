use core::ffi::{c_char, c_int};
use super::errno::ENOSYS;

#[repr(C)]
pub struct Group {
    name: *mut c_char,
    group_id: u32,
    members: *mut *mut c_char,
}

// FIXME: The kernel and runtime have no group database or group enumeration service.
fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub unsafe extern "C" fn endgrent() {
    set_errno(ENOSYS);
}

#[no_mangle]
pub unsafe extern "C" fn getgrent() -> *mut Group {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getgrgid(_group_id: u32) -> *mut Group {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getgrgid_r(
    _group_id: u32,
    _buffer: *mut Group,
    _scratch: *mut c_char,
    _scratch_size: usize,
    result: *mut *mut Group,
) -> c_int {
    if !result.is_null() {
        unsafe { result.write(core::ptr::null_mut()) };
    }
    ENOSYS
}

#[no_mangle]
pub unsafe extern "C" fn getgrnam(_name: *const c_char) -> *mut Group {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getgrnam_r(
    _name: *const c_char,
    _buffer: *mut Group,
    _scratch: *mut c_char,
    _scratch_size: usize,
    result: *mut *mut Group,
) -> c_int {
    if !result.is_null() {
        unsafe { result.write(core::ptr::null_mut()) };
    }
    ENOSYS
}

#[no_mangle]
pub unsafe extern "C" fn setgrent() {
    set_errno(ENOSYS);
}