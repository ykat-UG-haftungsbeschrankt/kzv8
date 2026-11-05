/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[allow(unused_imports)]
use core::ffi::{c_char, c_int, c_uint, c_void};
use super::super::errno::ENOSYS;

#[repr(C)]
pub struct IfNameIndex {
    interface_index: c_uint,
    interface_name: *mut c_char,
}

// FIXME: The kernel has no global POSIX interface-name registry or lookup API.
fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub unsafe extern "C" fn if_freenameindex(_interfaces: *mut IfNameIndex) {
    set_errno(ENOSYS);
}

#[no_mangle]
pub unsafe extern "C" fn if_indextoname(
    _interface_index: c_uint,
    _name: *mut c_char,
) -> *mut c_char {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn if_nameindex() -> *mut IfNameIndex {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn if_nametoindex(_name: *const c_char) -> c_uint {
    set_errno(ENOSYS);
    0
}