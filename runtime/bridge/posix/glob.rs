use core::ffi::{c_char, c_int, c_void};
use super::errno::ENOSYS;

const GLOB_ABORTED: c_int = 1;

#[repr(C)]
pub struct Glob {
    path_count: usize,
    path_vector: *mut *mut c_char,
    path_offset: usize,
}

unsafe extern "C" {
    fn free(pointer: *mut c_void);
}

// FIXME: The filesystem has no directory-entry iterator for pathname expansion.
#[no_mangle]
pub unsafe extern "C" fn glob(
    _pattern: *const c_char,
    _flags: c_int,
    _error_function: Option<unsafe extern "C" fn(*const c_char, c_int) -> c_int>,
    _results: *mut Glob,
) -> c_int {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    GLOB_ABORTED
}

#[no_mangle]
pub unsafe extern "C" fn globfree(results: *mut Glob) {
    if results.is_null() {
        return;
    }
    let path_count = unsafe { (*results).path_count };
    let path_offset = unsafe { (*results).path_offset };
    let path_vector = unsafe { (*results).path_vector };
    if !path_vector.is_null() {
        for index in path_offset..path_offset.saturating_add(path_count) {
            let path = unsafe { path_vector.add(index).read() };
            unsafe { free(path.cast()) };
        }
        unsafe { free(path_vector.cast()) };
        unsafe { (*results).path_vector = core::ptr::null_mut() };
    }
    unsafe {
        (*results).path_count = 0;
        (*results).path_offset = 0;
    }
}