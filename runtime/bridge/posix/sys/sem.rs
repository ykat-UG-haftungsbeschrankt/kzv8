use core::ffi::{c_int, c_void};
use super::super::errno::ENOSYS;

fn unavailable() -> c_int {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    -1
}

// FIXME: The kernel has no SysV semaphore sets or IPC identifier namespace.
#[no_mangle]
pub unsafe extern "C" fn semget(_key: i32, _semaphore_count: c_int, _flags: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn semop(
    _semaphore_id: c_int,
    _operations: *mut c_void,
    _operation_count: usize,
) -> c_int {
    unavailable()
}