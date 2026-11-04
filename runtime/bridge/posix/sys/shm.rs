use core::ffi::{c_int, c_void};
use super::super::errno::ENOSYS;

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

// FIXME: The kernel has no SysV shared-memory segment or attach API.
#[no_mangle]
pub unsafe extern "C" fn shmat(
    _shared_memory_id: c_int,
    _address: *const c_void,
    _flags: c_int,
) -> *mut c_void {
    set_errno(ENOSYS);
    usize::MAX as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn shmctl(
    _shared_memory_id: c_int,
    _command: c_int,
    _buffer: *mut c_void,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn shmdt(_address: *const c_void) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn shmget(_key: i32, _size: usize, _flags: c_int) -> c_int {
    set_errno(ENOSYS);
    -1
}