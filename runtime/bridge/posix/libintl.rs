use core::ffi::{c_char, c_int, c_ulong, c_void};
use super::errno::ENOSYS;

static DEFAULT_DOMAIN: &[u8] = b"messages\0";

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

fn translate(message: *const c_char) -> *mut c_char {
    message.cast_mut()
}

fn translate_plural(singular: *const c_char, plural: *const c_char, count: c_ulong) -> *mut c_char {
    if count == 1 {
        singular.cast_mut()
    } else {
        plural.cast_mut()
    }
}

#[no_mangle]
pub unsafe extern "C" fn bindtextdomain(
    _domain: *const c_char,
    _directory: *const c_char,
) -> *mut c_char {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn bind_textdomain_codeset(
    _domain: *const c_char,
    _codeset: *const c_char,
) -> *mut c_char {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn dcgettext(
    _domain: *const c_char,
    message: *const c_char,
    _category: c_int,
) -> *mut c_char {
    translate(message)
}

#[no_mangle]
pub unsafe extern "C" fn dcgettext_l(
    _domain: *const c_char,
    message: *const c_char,
    _category: c_int,
    _locale: *mut c_void,
) -> *mut c_char {
    translate(message)
}

#[no_mangle]
pub unsafe extern "C" fn dcngettext(
    _domain: *const c_char,
    singular: *const c_char,
    plural: *const c_char,
    count: c_ulong,
    _category: c_int,
) -> *mut c_char {
    translate_plural(singular, plural, count)
}

#[no_mangle]
pub unsafe extern "C" fn dcngettext_l(
    _domain: *const c_char,
    singular: *const c_char,
    plural: *const c_char,
    count: c_ulong,
    _category: c_int,
    _locale: *mut c_void,
) -> *mut c_char {
    translate_plural(singular, plural, count)
}

#[no_mangle]
pub unsafe extern "C" fn dgettext(
    _domain: *const c_char,
    message: *const c_char,
) -> *mut c_char {
    translate(message)
}

#[no_mangle]
pub unsafe extern "C" fn dgettext_l(
    _domain: *const c_char,
    message: *const c_char,
    _locale: *mut c_void,
) -> *mut c_char {
    translate(message)
}

#[no_mangle]
pub unsafe extern "C" fn dngettext(
    _domain: *const c_char,
    singular: *const c_char,
    plural: *const c_char,
    count: c_ulong,
) -> *mut c_char {
    translate_plural(singular, plural, count)
}

#[no_mangle]
pub unsafe extern "C" fn dngettext_l(
    _domain: *const c_char,
    singular: *const c_char,
    plural: *const c_char,
    count: c_ulong,
    _locale: *mut c_void,
) -> *mut c_char {
    translate_plural(singular, plural, count)
}

#[no_mangle]
pub unsafe extern "C" fn gettext(message: *const c_char) -> *mut c_char {
    translate(message)
}

#[no_mangle]
pub unsafe extern "C" fn gettext_l(message: *const c_char, _locale: *mut c_void) -> *mut c_char {
    translate(message)
}

#[no_mangle]
pub unsafe extern "C" fn ngettext(
    singular: *const c_char,
    plural: *const c_char,
    count: c_ulong,
) -> *mut c_char {
    translate_plural(singular, plural, count)
}

#[no_mangle]
pub unsafe extern "C" fn ngettext_l(
    singular: *const c_char,
    plural: *const c_char,
    count: c_ulong,
    _locale: *mut c_void,
) -> *mut c_char {
    translate_plural(singular, plural, count)
}

#[no_mangle]
pub unsafe extern "C" fn textdomain(domain: *const c_char) -> *mut c_char {
    if domain.is_null() {
        DEFAULT_DOMAIN.as_ptr().cast_mut().cast()
    } else {
        domain.cast_mut()
    }
}