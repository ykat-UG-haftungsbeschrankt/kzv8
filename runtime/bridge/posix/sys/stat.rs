use core::ffi::{c_char, c_int, c_void};
use core::ptr::read_volatile;
use super::super::errno::ENOSYS;

const AT_FDCWD: c_int = -100;

unsafe extern "C" {
    fn stat(path: *const c_char, status: *mut c_void) -> c_int;
    fn mkdir(path: *const c_char, mode: u32) -> c_int;
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

unsafe fn is_absolute(path: *const c_char) -> bool {
    !path.is_null() && unsafe { read_volatile(path.cast::<u8>()) } == b'/'
}

// FIXME: The filesystem has no chmod, device-node, FIFO, or timestamp mutation API.
#[no_mangle]
pub unsafe extern "C" fn chmod(_path: *const c_char, _mode: u32) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn fchmod(_file_descriptor: c_int, _mode: u32) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn fchmodat(
    _directory_descriptor: c_int,
    _path: *const c_char,
    _mode: u32,
    _flags: c_int,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn fstatat(
    directory_descriptor: c_int,
    path: *const c_char,
    status: *mut c_void,
    flags: c_int,
) -> c_int {
    if directory_descriptor == AT_FDCWD || unsafe { is_absolute(path) } {
        if flags & !0x100 != 0 {
            set_errno(ENOSYS);
            return -1;
        }
        return unsafe { stat(path, status) };
    }
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn futimens(_file_descriptor: c_int, _times: *const c_void) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn lstat(path: *const c_char, status: *mut c_void) -> c_int {
    unsafe { stat(path, status) }
}

#[no_mangle]
pub unsafe extern "C" fn mkdirat(
    directory_descriptor: c_int,
    path: *const c_char,
    mode: u32,
) -> c_int {
    if directory_descriptor == AT_FDCWD || unsafe { is_absolute(path) } {
        return unsafe { mkdir(path, mode) };
    }
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn mkfifo(_path: *const c_char, _mode: u32) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn mkfifoat(
    _directory_descriptor: c_int,
    _path: *const c_char,
    _mode: u32,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn mknod(
    _path: *const c_char,
    _mode: u32,
    _device: u64,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn mknodat(
    _directory_descriptor: c_int,
    _path: *const c_char,
    _mode: u32,
    _device: u64,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub extern "C" fn umask(_mask: u32) -> u32 {
    set_errno(ENOSYS);
    u32::MAX
}

#[no_mangle]
pub unsafe extern "C" fn utimensat(
    _directory_descriptor: c_int,
    _path: *const c_char,
    _times: *const c_void,
    _flags: c_int,
) -> c_int {
    set_errno(ENOSYS);
    -1
}