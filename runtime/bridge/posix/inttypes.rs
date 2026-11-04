use core::ffi::{c_char, c_int};
use core::ptr::read_volatile;
use super::errno::{EINVAL, ERANGE};

#[repr(C)]
pub struct Imaxdiv {
    quot: i64,
    rem: i64,
}

unsafe extern "C" {
    fn __isoc23_strtoll(text: *const c_char, end: *mut *mut c_char, base: c_int) -> i64;
    fn __isoc23_strtoull(text: *const c_char, end: *mut *mut c_char, base: c_int) -> u64;
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub extern "C" fn imaxabs(value: i64) -> i64 {
    value.wrapping_abs()
}

#[no_mangle]
pub extern "C" fn imaxdiv(numerator: i64, denominator: i64) -> Imaxdiv {
    if denominator == 0 || (numerator == i64::MIN && denominator == -1) {
        Imaxdiv { quot: 0, rem: 0 }
    } else {
        Imaxdiv {
            quot: numerator / denominator,
            rem: numerator % denominator,
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn strtoimax(
    text: *const c_char,
    end: *mut *mut c_char,
    base: c_int,
) -> i64 {
    unsafe { __isoc23_strtoll(text, end, base) }
}

#[no_mangle]
pub unsafe extern "C" fn strtoumax(
    text: *const c_char,
    end: *mut *mut c_char,
    base: c_int,
) -> u64 {
    unsafe { __isoc23_strtoull(text, end, base) }
}

fn wide_digit(character: i32) -> Option<u32> {
    match character {
        0x30..=0x39 => Some((character - 0x30) as u32),
        0x61..=0x7a => Some((character - 0x61 + 10) as u32),
        0x41..=0x5a => Some((character - 0x41 + 10) as u32),
        _ => None,
    }
}

unsafe fn parse_wide(
    text: *const i32,
    end: *mut *mut i32,
    base: c_int,
    signed: bool,
) -> (u64, bool, bool) {
    let start = text;
    let mut cursor = text;
    while matches!(unsafe { read_volatile(cursor) }, 0x20 | 0x09..=0x0d) {
        cursor = unsafe { cursor.add(1) };
    }

    let negative = unsafe { read_volatile(cursor) } == 0x2d;
    if negative || unsafe { read_volatile(cursor) } == 0x2b {
        cursor = unsafe { cursor.add(1) };
    }

    let mut radix = base;
    if !(radix == 0 || (2..=36).contains(&radix)) {
        set_errno(EINVAL);
        if !end.is_null() {
            unsafe { end.write(start.cast_mut()) };
        }
        return (0, negative, false);
    }
    if (radix == 0 || radix == 16)
        && unsafe { read_volatile(cursor) } == 0x30
        && matches!(unsafe { read_volatile(cursor.add(1)) }, 0x78 | 0x58)
        && wide_digit(unsafe { read_volatile(cursor.add(2)) }).is_some_and(|digit| digit < 16)
    {
        radix = 16;
        cursor = unsafe { cursor.add(2) };
    } else if radix == 0 {
        radix = if unsafe { read_volatile(cursor) } == 0x30 { 8 } else { 10 };
    }

    let digit_start = cursor;
    let limit = if signed {
        if negative { (i64::MAX as u64) + 1 } else { i64::MAX as u64 }
    } else {
        u64::MAX
    };
    let mut value = 0u64;
    let mut overflow = false;
    loop {
        let Some(digit) = wide_digit(unsafe { read_volatile(cursor) }) else {
            break;
        };
        if digit >= radix as u32 {
            break;
        }
        if value > (limit - digit as u64) / radix as u64 {
            overflow = true;
            value = limit;
        } else if !overflow {
            value = value * radix as u64 + digit as u64;
        }
        cursor = unsafe { cursor.add(1) };
    }

    if cursor == digit_start {
        if !end.is_null() {
            unsafe { end.write(start.cast_mut()) };
        }
        return (0, negative, false);
    }
    if !end.is_null() {
        unsafe { end.write(cursor.cast_mut()) };
    }
    if overflow {
        set_errno(ERANGE);
    }
    (value, negative, true)
}

#[no_mangle]
pub unsafe extern "C" fn wcstoimax(
    text: *const i32,
    end: *mut *mut i32,
    base: c_int,
) -> i64 {
    let (value, negative, _) = unsafe { parse_wide(text, end, base, true) };
    if negative {
        0u64.wrapping_sub(value) as i64
    } else {
        value as i64
    }
}

#[no_mangle]
pub unsafe extern "C" fn wcstoumax(
    text: *const i32,
    end: *mut *mut i32,
    base: c_int,
) -> u64 {
    let (value, negative, _) = unsafe { parse_wide(text, end, base, false) };
    if negative {
        0u64.wrapping_sub(value)
    } else {
        value
    }
}