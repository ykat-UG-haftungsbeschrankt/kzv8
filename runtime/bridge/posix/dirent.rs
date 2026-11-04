use core::ffi::{c_char, c_int, c_long, c_void};
use core::ptr::read_volatile;
use super::errno::ENOSYS;

#[repr(C)]
pub struct Dirent {
    inode: u64,
    name: [c_char; 256],
}

// FIXME: The filesystem has no directory-stream handle or directory-entry iteration API.
fn unavailable() -> c_int {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    -1
}

unsafe fn compare_names(left: *const Dirent, right: *const Dirent) -> c_int {
    for index in 0..256 {
        let left_byte = unsafe { read_volatile((*left).name.as_ptr().add(index).cast::<u8>()) };
        let right_byte = unsafe { read_volatile((*right).name.as_ptr().add(index).cast::<u8>()) };
        if left_byte != right_byte {
            return left_byte as c_int - right_byte as c_int;
        }
        if left_byte == 0 {
            return 0;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn alphasort(
    left: *const *const Dirent,
    right: *const *const Dirent,
) -> c_int {
    if left.is_null() || right.is_null() {
        return match (left.is_null(), right.is_null()) {
            (true, true) => 0,
            (true, false) => -1,
            (false, true) => 1,
            (false, false) => 0,
        };
    }
    let left_entry = unsafe { read_volatile(left) };
    let right_entry = unsafe { read_volatile(right) };
    if left_entry.is_null() || right_entry.is_null() {
        return match (left_entry.is_null(), right_entry.is_null()) {
            (true, true) => 0,
            (true, false) => -1,
            (false, true) => 1,
            (false, false) => 0,
        };
    }
    unsafe { compare_names(left_entry, right_entry) }
}

#[no_mangle]
pub unsafe extern "C" fn closedir(_directory: *mut c_void) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn dirfd(_directory: *mut c_void) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn fdopendir(_file_descriptor: c_int) -> *mut c_void {
    unavailable();
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn opendir(_path: *const c_char) -> *mut c_void {
    unavailable();
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn posix_getdents(
    _file_descriptor: c_int,
    _buffer: *mut c_void,
    _buffer_size: usize,
    _flags: c_int,
) -> isize {
    unavailable() as isize
}

#[no_mangle]
pub unsafe extern "C" fn readdir(_directory: *mut c_void) -> *mut Dirent {
    unavailable();
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn readdir_r(
    _directory: *mut c_void,
    _entry: *mut Dirent,
    _result: *mut *mut Dirent,
) -> c_int {
    ENOSYS
}

#[no_mangle]
pub unsafe extern "C" fn rewinddir(_directory: *mut c_void) {
    unavailable();
}

#[no_mangle]
pub unsafe extern "C" fn scandir(
    _path: *const c_char,
    _entries: *mut *mut *mut Dirent,
    _select: Option<unsafe extern "C" fn(*const Dirent) -> c_int>,
    _compare: Option<
        unsafe extern "C" fn(*const *const Dirent, *const *const Dirent) -> c_int,
    >,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn seekdir(_directory: *mut c_void, _position: c_long) {
    unavailable();
}

#[no_mangle]
pub unsafe extern "C" fn telldir(_directory: *mut c_void) -> c_long {
    unavailable() as c_long
}