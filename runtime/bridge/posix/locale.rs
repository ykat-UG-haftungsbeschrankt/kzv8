use core::ffi::{c_char, c_int, c_void};
use core::ptr::read_volatile;
use super::errno::EINVAL;

const CHAR_MAX: i8 = 127;

#[repr(C)]
struct LocaleConv {
    decimal_point: *mut c_char,
    thousands_sep: *mut c_char,
    grouping: *mut c_char,
    int_curr_symbol: *mut c_char,
    currency_symbol: *mut c_char,
    mon_decimal_point: *mut c_char,
    mon_thousands_sep: *mut c_char,
    mon_grouping: *mut c_char,
    positive_sign: *mut c_char,
    negative_sign: *mut c_char,
    int_frac_digits: i8,
    frac_digits: i8,
    p_cs_precedes: i8,
    p_sep_by_space: i8,
    n_cs_precedes: i8,
    n_sep_by_space: i8,
    p_sign_posn: i8,
    n_sign_posn: i8,
    int_p_cs_precedes: i8,
    int_p_sep_by_space: i8,
    int_n_cs_precedes: i8,
    int_n_sep_by_space: i8,
    int_p_sign_posn: i8,
    int_n_sign_posn: i8,
}

const EMPTY: *mut c_char = b"\0".as_ptr() as *mut c_char;
const C_LOCALE_NAME: *const c_char = b"C\0".as_ptr().cast();

static mut C_LOCALE_CONV: LocaleConv = LocaleConv {
    decimal_point: b".\0".as_ptr() as *mut c_char,
    thousands_sep: EMPTY,
    grouping: EMPTY,
    int_curr_symbol: EMPTY,
    currency_symbol: EMPTY,
    mon_decimal_point: EMPTY,
    mon_thousands_sep: EMPTY,
    mon_grouping: EMPTY,
    positive_sign: EMPTY,
    negative_sign: EMPTY,
    int_frac_digits: CHAR_MAX,
    frac_digits: CHAR_MAX,
    p_cs_precedes: CHAR_MAX,
    p_sep_by_space: CHAR_MAX,
    n_cs_precedes: CHAR_MAX,
    n_sep_by_space: CHAR_MAX,
    p_sign_posn: CHAR_MAX,
    n_sign_posn: CHAR_MAX,
    int_p_cs_precedes: CHAR_MAX,
    int_p_sep_by_space: CHAR_MAX,
    int_n_cs_precedes: CHAR_MAX,
    int_n_sep_by_space: CHAR_MAX,
    int_p_sign_posn: CHAR_MAX,
    int_n_sign_posn: CHAR_MAX,
};

unsafe extern "C" {
    fn uselocale(locale: *mut c_void) -> *mut c_void;
    fn __errno_location() -> *mut c_int;
}

fn set_errno(error: c_int) {
    unsafe { __errno_location().write(error) };
}

unsafe fn is_c_locale(name: *const c_char) -> bool {
    if name.is_null() {
        return false;
    }
    [b"C\0".as_slice(), b"POSIX\0", b"C.UTF-8\0", b"C.utf8\0"]
        .iter()
        .any(|candidate| unsafe { c_string_equals(name, candidate) })
}

unsafe fn c_string_equals(input: *const c_char, expected: &[u8]) -> bool {
    for (index, byte) in expected.iter().copied().enumerate() {
        if unsafe { read_volatile(input.cast::<u8>().add(index)) } != byte {
            return false;
        }
    }
    true
}

#[no_mangle]
pub unsafe extern "C" fn duplocale(locale: *mut c_void) -> *mut c_void {
    if locale.is_null() {
        set_errno(EINVAL);
        core::ptr::null_mut()
    } else if locale == usize::MAX as *mut c_void {
        unsafe { uselocale(core::ptr::null_mut()) }
    } else {
        locale
    }
}

#[no_mangle]
pub unsafe extern "C" fn getlocalename_l(
    category: c_int,
    locale: *mut c_void,
) -> *const c_char {
    if !(0..=6).contains(&category) || locale.is_null() {
        set_errno(EINVAL);
        return core::ptr::null();
    }
    C_LOCALE_NAME
}

#[no_mangle]
pub extern "C" fn localeconv() -> *mut LocaleConv {
    core::ptr::addr_of_mut!(C_LOCALE_CONV)
}

#[no_mangle]
pub unsafe extern "C" fn setlocale(category: c_int, locale_name: *const c_char) -> *mut c_char {
    if !(0..=6).contains(&category) {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    if locale_name.is_null() || unsafe { is_c_locale(locale_name) } {
        return C_LOCALE_NAME.cast_mut();
    }
    set_errno(EINVAL);
    core::ptr::null_mut()
}