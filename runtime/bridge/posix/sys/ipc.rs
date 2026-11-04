use core::ffi::{c_char, c_int};
use super::super::errno::ENOSYS;

// FIXME: The filesystem has no stable inode/device identifiers for IPC key generation.
#[no_mangle]
pub unsafe extern "C" fn ftok(_path: *const c_char, _project_id: c_int) -> i32 {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    -1
}