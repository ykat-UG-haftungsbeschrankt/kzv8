use core::ffi::{c_char, c_int, c_void};

const WRDE_NOSYS: c_int = 6;

// FIXME: The runtime has no shell or process environment for word expansion.
#[no_mangle]
pub unsafe extern "C" fn wordexp(
    _words: *const c_char,
    _word_expansion: *mut c_void,
    _flags: c_int,
) -> c_int {
    WRDE_NOSYS
}

#[no_mangle]
pub unsafe extern "C" fn wordfree(_word_expansion: *mut c_void) {}