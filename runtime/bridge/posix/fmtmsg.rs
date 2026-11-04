use core::ffi::{c_char, c_int, c_long};

const MM_NOTOK: c_int = 1;

// FIXME: The kernel has no formatted diagnostic or classified console-message interface.
#[no_mangle]
pub unsafe extern "C" fn fmtmsg(
    _classification: c_long,
    _label: *const c_char,
    _severity: c_int,
    _text: *const c_char,
    _action: *const c_char,
    _tag: *const c_char,
) -> c_int {
    MM_NOTOK
}