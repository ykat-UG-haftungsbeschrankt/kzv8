use core::ffi::{c_int, c_void};
use super::super::errno::ENOSYS;

// FIXME: The kernel tracks no per-process or child CPU time counters.
#[no_mangle]
pub unsafe extern "C" fn times(_buffer: *mut c_void) -> i64 {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS as c_int);
    }
    -1
}