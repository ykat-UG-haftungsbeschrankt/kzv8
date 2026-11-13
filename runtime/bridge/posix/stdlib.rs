/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_char, c_int, c_long, c_longlong, c_void};
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};
use crate::kernel::dl::AddressSpace;
use super::errno::{as_errno, EFAULT, ENOSYS, ENOMEM, ERANGE};

const RAND_MAX: c_int = 32767;
static RANDOM_STATE: AtomicU32 = AtomicU32::new(1);
static mut PTSNAME_BUFFER: [u8; 32] = [0; 32];

type PlainCompare = Option<unsafe extern "C" fn(*const c_void, *const c_void) -> c_int>;
type ContextCompare = Option<
    unsafe extern "C" fn(*const c_void, *const c_void, *mut c_void) -> c_int,
>;

enum Comparator {
    Plain(PlainCompare),
    Context(ContextCompare, *mut c_void),
}

unsafe fn compare(
    comparator: &Comparator,
    left: *const u8,
    right: *const u8,
) -> c_int {
    match comparator {
        Comparator::Plain(Some(function)) => unsafe { function(left.cast(), right.cast()) },
        Comparator::Context(Some(function), context) => unsafe {
            function(left.cast(), right.cast(), *context)
        },
        _ => 0,
    }
}

unsafe fn swap_elements(left: *mut u8, right: *mut u8, size: usize) {
    for index in 0..size {
        let left_byte = unsafe { read_volatile(left.add(index)) };
        let right_byte = unsafe { read_volatile(right.add(index)) };
        unsafe {
            write_volatile(left.add(index), right_byte);
            write_volatile(right.add(index), left_byte);
        }
    }
}

unsafe fn sift_down(
    base: *mut u8,
    mut root: usize,
    end: usize,
    size: usize,
    comparator: &Comparator,
) {
    while root < end / 2 {
        let mut child = root * 2 + 1;
        if child + 1 < end
            && unsafe {
                compare(
                    comparator,
                    base.add(child * size),
                    base.add((child + 1) * size),
                )
            } < 0
        {
            child += 1;
        }
        if unsafe {
            compare(comparator, base.add(root * size), base.add(child * size))
        } >= 0
        {
            return;
        }
        unsafe { swap_elements(base.add(root * size), base.add(child * size), size) };
        root = child;
    }
}

unsafe fn heap_sort(base: *mut c_void, count: usize, size: usize, comparator: Comparator) {
    if base.is_null() || size == 0 || count < 2 {
        return;
    }
    let base = base.cast::<u8>();
    for root in (0..count / 2).rev() {
        unsafe { sift_down(base, root, count, size, &comparator) };
    }
    for end in (1..count).rev() {
        unsafe {
            swap_elements(base, base.add(end * size), size);
            sift_down(base, 0, end, size, &comparator);
        }
    }
}

#[no_mangle]
pub extern "C" fn abs(value: c_int) -> c_int {
    value.wrapping_abs()
}

#[no_mangle]
pub extern "C" fn labs(value: c_long) -> c_long {
    value.wrapping_abs()
}

#[no_mangle]
pub extern "C" fn llabs(value: c_longlong) -> c_longlong {
    value.wrapping_abs()
}

#[repr(C)]
pub struct Div {
    quotient: c_int,
    remainder: c_int,
}

#[repr(C)]
pub struct Ldiv {
    quotient: c_long,
    remainder: c_long,
}

#[repr(C)]
pub struct Lldiv {
    quotient: c_longlong,
    remainder: c_longlong,
}

#[no_mangle]
pub extern "C" fn div(numerator: c_int, denominator: c_int) -> Div {
    if denominator == 0 || (numerator == c_int::MIN && denominator == -1) {
        Div { quotient: 0, remainder: 0 }
    } else {
        Div { quotient: numerator / denominator, remainder: numerator % denominator }
    }
}

#[no_mangle]
pub extern "C" fn ldiv(numerator: c_long, denominator: c_long) -> Ldiv {
    if denominator == 0 || (numerator == c_long::MIN && denominator == -1) {
        Ldiv { quotient: 0, remainder: 0 }
    } else {
        Ldiv { quotient: numerator / denominator, remainder: numerator % denominator }
    }
}

#[no_mangle]
pub extern "C" fn lldiv(numerator: c_longlong, denominator: c_longlong) -> Lldiv {
    if denominator == 0 || (numerator == c_longlong::MIN && denominator == -1) {
        Lldiv { quotient: 0, remainder: 0 }
    } else {
        Lldiv { quotient: numerator / denominator, remainder: numerator % denominator }
    }
}

#[no_mangle]
pub unsafe extern "C" fn bsearch(
    key: *const c_void,
    base: *const c_void,
    count: usize,
    size: usize,
    compare: PlainCompare,
) -> *mut c_void {
    let Some(compare) = compare else {
        return core::ptr::null_mut();
    };
    if key.is_null() || base.is_null() || size == 0 {
        return core::ptr::null_mut();
    }
    let mut lower = 0usize;
    let mut upper = count;
    while lower < upper {
        let middle = lower + (upper - lower) / 2;
        let candidate = unsafe { base.cast::<u8>().add(middle * size) };
        match unsafe { compare(key, candidate.cast()) } {
            0 => return candidate.cast_mut().cast(),
            result if result < 0 => upper = middle,
            _ => lower = middle + 1,
        }
    }
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn qsort(
    base: *mut c_void,
    count: usize,
    size: usize,
    compare: PlainCompare,
) {
    unsafe { heap_sort(base, count, size, Comparator::Plain(compare)) };
}

#[no_mangle]
pub unsafe extern "C" fn qsort_r(
    base: *mut c_void,
    count: usize,
    size: usize,
    compare: ContextCompare,
    context: *mut c_void,
) {
    unsafe { heap_sort(base, count, size, Comparator::Context(compare, context)) };
}

#[no_mangle]
pub extern "C" fn srand(seed: u32) {
    RANDOM_STATE.store(seed, Ordering::Relaxed);
}

fn next_random() -> u32 {
    let mut current = RANDOM_STATE.load(Ordering::Relaxed);
    loop {
        let next = current.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        match RANDOM_STATE.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(actual) => current = actual,
        }
    }
}

#[no_mangle]
pub extern "C" fn rand() -> c_int {
    ((next_random() >> 16) & RAND_MAX as u32) as c_int
}

#[no_mangle]
pub unsafe extern "C" fn rand_r(seed: *mut u32) -> c_int {
    if seed.is_null() {
        return 0;
    }
    let state = unsafe { seed.read() }.wrapping_mul(1_103_515_245).wrapping_add(12_345);
    unsafe { seed.write(state) };
    ((state >> 16) & RAND_MAX as u32) as c_int
}

#[no_mangle]
pub extern "C" fn random() -> c_long {
    next_random() as c_long
}

#[no_mangle]
pub unsafe extern "C" fn reallocarray(
    pointer: *mut c_void,
    count: usize,
    size: usize,
) -> *mut c_void {
    let Some(total) = count.checked_mul(size) else {
        set_errno(ENOMEM);
        return core::ptr::null_mut();
    };
    unsafe { realloc(pointer, total) }
}

unsafe extern "C" {
    fn realloc(pointer: *mut c_void, size: usize) -> *mut c_void;
    fn getenv(name: *const c_char) -> *mut c_char;
    fn __isoc23_strtol(text: *const c_char, end: *mut *mut c_char, base: c_int) -> isize;
    fn __isoc23_strtoll(text: *const c_char, end: *mut *mut c_char, base: c_int) -> i64;
    fn strtod(text: *const c_char, end: *mut *mut c_char) -> f64;
    #[allow(dead_code)]
    fn strtof(text: *const c_char, end: *mut *mut c_char) -> f32;
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub extern "C" fn atoi(text: *const c_char) -> c_int {
    unsafe { __isoc23_strtol(text, core::ptr::null_mut(), 10) as c_int }
}

#[no_mangle]
pub extern "C" fn atol(text: *const c_char) -> c_long {
    unsafe { __isoc23_strtol(text, core::ptr::null_mut(), 10) as c_long }
}

#[no_mangle]
pub extern "C" fn atoll(text: *const c_char) -> c_longlong {
    unsafe { __isoc23_strtoll(text, core::ptr::null_mut(), 10) as c_longlong }
}

#[no_mangle]
pub extern "C" fn atof(text: *const c_char) -> f64 {
    unsafe { strtod(text, core::ptr::null_mut()) }
}

#[no_mangle]
pub unsafe extern "C" fn strtol(
    text: *const c_char,
    end: *mut *mut c_char,
    base: c_int,
) -> isize {
    unsafe { __isoc23_strtol(text, end, base) }
}

#[no_mangle]
pub unsafe extern "C" fn strtoll(
    text: *const c_char,
    end: *mut *mut c_char,
    base: c_int,
) -> i64 {
    unsafe { __isoc23_strtoll(text, end, base) }
}

#[no_mangle]
pub unsafe extern "C" fn secure_getenv(name: *const c_char) -> *mut c_char {
    unsafe { getenv(name) }
}

// FIXME: The runtime has no process environment, shell, or secure temporary-file namespace.
#[no_mangle]
pub unsafe extern "C" fn setenv(
    _name: *const c_char,
    _value: *const c_char,
    _overwrite: c_int,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn unsetenv(_name: *const c_char) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn putenv(_string: *mut c_char) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn system(_command: *const c_char) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn realpath(
    _path: *const c_char,
    _resolved_path: *mut c_char,
) -> *mut c_char {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn mkstemp(_template_name: *mut c_char) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn mkostemp(_template_name: *mut c_char, _flags: c_int) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn mkdtemp(_template_name: *mut c_char) -> *mut c_char {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

fn format_pty_name(index: usize, output: &mut [u8; 32]) -> usize {
    let prefix = b"/dev/pts/";
    output[..prefix.len()].copy_from_slice(prefix);
    let mut digits = [0; usize::BITS as usize / 3 + 1];
    let mut value = index;
    let mut digit_count = 0;
    loop {
        digits[digit_count] = b'0' + (value % 10) as u8;
        digit_count += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    for (offset, digit) in digits[..digit_count].iter().rev().enumerate() {
        output[prefix.len() + offset] = *digit;
    }
    let length = prefix.len() + digit_count;
    output[length] = 0;
    length + 1
}

fn pty_error(error: crate::kernel::error::Error) -> c_int {
    as_errno(error)
}

#[no_mangle]
pub unsafe extern "C" fn posix_openpt(flags: c_int) -> c_int {
    match crate::kernel::runtime::runtime_libc::pty::open_master(flags) {
        Ok(file_descriptor) => file_descriptor,
        Err(error) => {
            set_errno(pty_error(error));
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn grantpt(file_descriptor: c_int) -> c_int {
    match crate::kernel::runtime::runtime_libc::pty::pty_index(file_descriptor) {
        Ok(_) => 0,
        Err(error) => {
            set_errno(pty_error(error));
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn unlockpt(file_descriptor: c_int) -> c_int {
    match crate::kernel::runtime::runtime_libc::pty::unlock_slave(file_descriptor) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(pty_error(error));
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn ptsname(file_descriptor: c_int) -> *mut c_char {
    let index = match crate::kernel::runtime::runtime_libc::pty::pty_index(file_descriptor) {
        Ok(index) => index,
        Err(error) => {
            set_errno(pty_error(error));
            return core::ptr::null_mut();
        }
    };
    let buffer = unsafe { &mut *core::ptr::addr_of_mut!(PTSNAME_BUFFER) };
    let _length = format_pty_name(index, buffer);
    buffer.as_mut_ptr().cast()
}

#[no_mangle]
pub unsafe extern "C" fn ptsname_r(
    file_descriptor: c_int,
    buffer: *mut c_char,
    buffer_size: usize,
) -> c_int {
    if buffer.is_null() {
        return EFAULT;
    }
    let index = match crate::kernel::runtime::runtime_libc::pty::pty_index(file_descriptor) {
        Ok(index) => index,
        Err(error) => return pty_error(error),
    };
    let mut name = [0; 32];
    let length = format_pty_name(index, &mut name);
    if buffer_size < length {
        return ERANGE;
    }
    let process = match crate::kernel::scheduler::thread::ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => return pty_error(error),
    };
    match process.address_space().write(buffer as usize, &name[..length]) {
        Ok(()) => 0,
        Err(error) => pty_error(error),
    }
}