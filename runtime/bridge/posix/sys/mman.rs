use core::ffi::{c_char, c_int, c_void};
use super::super::errno::ENOSYS;

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

// FIXME: The kernel has no page pinning, backing-store sync, or POSIX shared-memory object API.
#[no_mangle]
pub unsafe extern "C" fn mlock(_address: *const c_void, _length: usize) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn mlockall(_flags: c_int) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn msync(_address: *mut c_void, _length: usize, _flags: c_int) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn munlock(_address: *const c_void, _length: usize) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn munlockall() -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn posix_madvise(
    _address: *mut c_void,
    _length: usize,
    _advice: c_int,
) -> c_int {
    ENOSYS
}

#[no_mangle]
pub unsafe extern "C" fn shm_open(
    _name: *const c_char,
    _flags: c_int,
    _mode: u32,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn shm_unlink(_name: *const c_char) -> c_int {
    set_errno(ENOSYS);
    -1
}