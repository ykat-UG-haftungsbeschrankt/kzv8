use core::ffi::{c_char, c_int, c_long, c_longlong, c_void};
use core::ptr::read_volatile;

unsafe fn compare_folded(left: *const u8, right: *const u8, count: Option<usize>) -> c_int {
    let mut index = 0;
    loop {
        if count.is_some_and(|limit| index == limit) {
            return 0;
        }
        let left_byte = unsafe { read_volatile(left.add(index)) };
        let right_byte = unsafe { read_volatile(right.add(index)) };
        let folded_left = left_byte.to_ascii_lowercase();
        let folded_right = right_byte.to_ascii_lowercase();
        if folded_left != folded_right {
            return folded_left as c_int - folded_right as c_int;
        }
        if left_byte == 0 {
            return 0;
        }
        index += 1;
    }
}

#[no_mangle]
pub extern "C" fn ffs(value: c_int) -> c_int {
    if value == 0 { 0 } else { value.trailing_zeros() as c_int + 1 }
}

#[no_mangle]
pub extern "C" fn ffsl(value: c_long) -> c_int {
    if value == 0 { 0 } else { value.trailing_zeros() as c_int + 1 }
}

#[no_mangle]
pub extern "C" fn ffsll(value: c_longlong) -> c_int {
    if value == 0 { 0 } else { value.trailing_zeros() as c_int + 1 }
}

#[no_mangle]
pub unsafe extern "C" fn strcasecmp(left: *const c_char, right: *const c_char) -> c_int {
    unsafe { compare_folded(left.cast(), right.cast(), None) }
}

#[no_mangle]
pub unsafe extern "C" fn strcasecmp_l(
    left: *const c_char,
    right: *const c_char,
    _locale: *mut c_void,
) -> c_int {
    unsafe { compare_folded(left.cast(), right.cast(), None) }
}

#[no_mangle]
pub unsafe extern "C" fn strncasecmp(
    left: *const c_char,
    right: *const c_char,
    count: usize,
) -> c_int {
    unsafe { compare_folded(left.cast(), right.cast(), Some(count)) }
}

#[no_mangle]
pub unsafe extern "C" fn strncasecmp_l(
    left: *const c_char,
    right: *const c_char,
    count: usize,
    _locale: *mut c_void,
) -> c_int {
    unsafe { compare_folded(left.cast(), right.cast(), Some(count)) }
}