use core::ffi::{c_char, c_int, c_void};
use core::ptr::copy_nonoverlapping;

const REG_NOMATCH: c_int = 1;
const REG_ENOSYS: c_int = 14;

#[repr(C)]
pub struct Regex {
    subexpressions: usize,
    opaque: [*mut c_void; 8],
}

// FIXME: No POSIX-compatible regular-expression engine is linked into the runtime.
#[no_mangle]
pub unsafe extern "C" fn regcomp(
    expression: *mut Regex,
    _pattern: *const c_char,
    _flags: c_int,
) -> c_int {
    if !expression.is_null() {
        unsafe {
            expression.write(Regex {
                subexpressions: 0,
                opaque: [core::ptr::null_mut(); 8],
            });
        }
    }
    REG_ENOSYS
}

#[no_mangle]
pub unsafe extern "C" fn regerror(
    error_code: c_int,
    _expression: *const Regex,
    buffer: *mut c_char,
    buffer_size: usize,
) -> usize {
    let message: &[u8] = if error_code == REG_ENOSYS {
        b"regular expressions are not supported\0"
    } else if error_code == REG_NOMATCH {
        b"regular expression did not match\0"
    } else {
        b"unknown regular expression error\0"
    };
    if !buffer.is_null() && buffer_size != 0 {
        let copied = message.len().min(buffer_size);
        unsafe {
            copy_nonoverlapping(message.as_ptr(), buffer.cast::<u8>(), copied);
            if copied == buffer_size {
                buffer.add(buffer_size - 1).write(0);
            }
        }
    }
    message.len()
}

#[no_mangle]
pub unsafe extern "C" fn regexec(
    _expression: *const Regex,
    _string: *const c_char,
    _match_count: usize,
    _matches: *mut c_void,
    _flags: c_int,
) -> c_int {
    REG_NOMATCH
}

#[no_mangle]
pub unsafe extern "C" fn regfree(expression: *mut Regex) {
    if !expression.is_null() {
        unsafe {
            expression.write(Regex {
                subexpressions: 0,
                opaque: [core::ptr::null_mut(); 8],
            });
        }
    }
}