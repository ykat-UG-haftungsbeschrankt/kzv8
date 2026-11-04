use core::ffi::{c_char, c_int};
use super::errno::ENOSYS;

const O_WRONLY: c_int = 1;
const O_CREAT: c_int = 0o100;
const O_TRUNC: c_int = 0o1000;

unsafe extern "C" {
    fn kz_rust_open(path: *const c_char, flags: c_int, mode: u32) -> c_int;
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub unsafe extern "C" fn creat(path: *const c_char, mode: u32) -> c_int {
    unsafe { kz_rust_open(path, O_WRONLY | O_CREAT | O_TRUNC, mode) }
}

// FIXME: The kernel has no descriptor duplication, flag, or record-lock operations.
#[no_mangle]
pub unsafe extern "C" fn fcntl(
    _file_descriptor: c_int,
    _command: c_int,
    _argument: usize,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

// FIXME: The filesystem has no directory-descriptor-relative path resolution.
#[no_mangle]
pub unsafe extern "C" fn openat(
    _directory_descriptor: c_int,
    _path: *const c_char,
    _flags: c_int,
    _mode: u32,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub extern "C" fn posix_fadvise(
    _file_descriptor: c_int,
    _offset: i64,
    _length: i64,
    _advice: c_int,
) -> c_int {
    ENOSYS
}

#[no_mangle]
pub extern "C" fn posix_fallocate(
    _file_descriptor: c_int,
    _offset: i64,
    _length: i64,
) -> c_int {
    ENOSYS
}