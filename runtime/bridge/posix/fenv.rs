use core::ffi::{c_int, c_void};
use super::errno::ENOSYS;

// FIXME: The kernel does not preserve per-thread floating-point control or status state.
fn unavailable() -> c_int {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn fegetenv(_environment: *mut c_void) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn fegetexceptflag(_flag: *mut u32, _exceptions: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub extern "C" fn fegetround() -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn feholdexcept(_environment: *mut c_void) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn fesetenv(_environment: *const c_void) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn fesetexceptflag(_flag: *const u32, _exceptions: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub extern "C" fn fesetround(_rounding_direction: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub extern "C" fn fetestexcept(_exceptions: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn feupdateenv(_environment: *const c_void) -> c_int {
    unavailable()
}