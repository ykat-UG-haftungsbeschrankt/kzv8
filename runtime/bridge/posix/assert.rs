use core::ffi::{c_char, c_uint};

unsafe extern "C" {
    fn abort() -> !;
}

#[no_mangle]
pub unsafe extern "C" fn __assert_fail(
    _expression: *const c_char,
    _file: *const c_char,
    _line: c_uint,
    _function: *const c_char,
) -> ! {
    unsafe { abort() }
}