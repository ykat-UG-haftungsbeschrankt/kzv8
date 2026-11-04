use core::ffi::{c_char, c_int, c_void};
use super::errno::ENOSYS;

#[repr(C)]
pub struct Ftw {
    base: c_int,
    level: c_int,
}

type WalkCallback =
    Option<unsafe extern "C" fn(*const c_char, *const c_void, c_int, *mut Ftw) -> c_int>;

// FIXME: The filesystem has no directory iteration API for file-tree traversal.
fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub extern "C" fn nftw(
    _path: *const c_char,
    _callback: WalkCallback,
    _descriptor_limit: c_int,
    _flags: c_int,
) -> c_int {
    set_errno(ENOSYS);
    -1
}