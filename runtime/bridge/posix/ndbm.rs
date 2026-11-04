use core::ffi::{c_char, c_int, c_void};
use super::errno::ENOSYS;

#[repr(C)]
pub struct Datum {
    data: *mut c_void,
    size: usize,
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

fn empty_datum() -> Datum {
    Datum {
        data: core::ptr::null_mut(),
        size: 0,
    }
}

// FIXME: The kernel and runtime have no DBM database or persistent key/value store.
#[no_mangle]
pub unsafe extern "C" fn dbm_clearerr(_database: *mut c_void) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn dbm_close(_database: *mut c_void) {
    set_errno(ENOSYS);
}

#[no_mangle]
pub unsafe extern "C" fn dbm_delete(_database: *mut c_void, _key: Datum) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn dbm_error(_database: *mut c_void) -> c_int {
    1
}

#[no_mangle]
pub unsafe extern "C" fn dbm_fetch(_database: *mut c_void, _key: Datum) -> Datum {
    set_errno(ENOSYS);
    empty_datum()
}

#[no_mangle]
pub unsafe extern "C" fn dbm_firstkey(_database: *mut c_void) -> Datum {
    set_errno(ENOSYS);
    empty_datum()
}

#[no_mangle]
pub unsafe extern "C" fn dbm_nextkey(_database: *mut c_void) -> Datum {
    set_errno(ENOSYS);
    empty_datum()
}

#[no_mangle]
pub unsafe extern "C" fn dbm_open(
    _file: *const c_char,
    _flags: c_int,
    _mode: u32,
) -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn dbm_store(
    _database: *mut c_void,
    _key: Datum,
    _content: Datum,
    _mode: c_int,
) -> c_int {
    set_errno(ENOSYS);
    -1
}