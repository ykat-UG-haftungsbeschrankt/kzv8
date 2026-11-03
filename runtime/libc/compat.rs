/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_char, c_void};
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU8, Ordering};

const EINVAL: i32 = 22;
const WEOF: u32 = u32::MAX;

#[repr(C)]
struct LocaleStub {
    active: i32,
}

static mut C_LOCALE: LocaleStub = LocaleStub { active: 1 };
static LOWER_STATE: AtomicU8 = AtomicU8::new(0);
static UPPER_STATE: AtomicU8 = AtomicU8::new(0);
static mut LOWER_TABLE: [i32; 384] = [0; 384];
static mut UPPER_TABLE: [i32; 384] = [0; 384];
static mut LOWER_POINTER: *const i32 = core::ptr::null();
static mut UPPER_POINTER: *const i32 = core::ptr::null();

unsafe fn c_string_equals(input: *const c_char, expected: &[u8]) -> bool {
    if input.is_null() {
        return false;
    }
    for (index, expected_byte) in expected.iter().copied().enumerate() {
        if unsafe { read_volatile(input.cast::<u8>().add(index)) } != expected_byte {
            return false;
        }
    }
    unsafe { read_volatile(input.cast::<u8>().add(expected.len())) == 0 }
}

#[no_mangle]
pub unsafe extern "C" fn newlocale(
    _category_mask: i32,
    locale_name: *const c_char,
    _base: *mut c_void,
) -> *mut c_void {
    if locale_name.is_null()
        || unsafe { c_string_equals(locale_name, b"C") }
        || unsafe { c_string_equals(locale_name, b"POSIX") }
        || unsafe { c_string_equals(locale_name, b"C.UTF-8") }
        || unsafe { c_string_equals(locale_name, b"C.utf8") }
    {
        core::ptr::addr_of_mut!(C_LOCALE).cast()
    } else {
        crate::kernel::runtime::runtime_libc::posix::set_errno(EINVAL);
        core::ptr::null_mut()
    }
}

#[no_mangle]
pub extern "C" fn freelocale(_locale: *mut c_void) {}

#[no_mangle]
pub extern "C" fn uselocale(locale: *mut c_void) -> *mut c_void {
    if locale.is_null() {
        core::ptr::addr_of_mut!(C_LOCALE).cast()
    } else {
        locale
    }
}

fn initialize_case_table(state: &AtomicU8, uppercase: bool) {
    if state
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        unsafe {
            let table = if uppercase {
                core::ptr::addr_of_mut!(UPPER_TABLE).cast::<i32>()
            } else {
                core::ptr::addr_of_mut!(LOWER_TABLE).cast::<i32>()
            };
            for value in 0..256usize {
                let byte = value as u8;
                let converted = if uppercase && byte.is_ascii_lowercase() {
                    (byte - b'a' + b'A') as i32
                } else if !uppercase && byte.is_ascii_uppercase() {
                    (byte - b'A' + b'a') as i32
                } else {
                    value as i32
                };
                unsafe { table.add(value + 128).write(converted) };
            }
            if uppercase {
                UPPER_POINTER = table.add(128);
            } else {
                LOWER_POINTER = table.add(128);
            }
        }
        state.store(2, Ordering::Release);
    } else {
        while state.load(Ordering::Acquire) != 2 {
            core::hint::spin_loop();
        }
    }
}

#[no_mangle]
pub extern "C" fn __ctype_tolower_loc() -> *mut *const i32 {
    initialize_case_table(&LOWER_STATE, false);
    core::ptr::addr_of_mut!(LOWER_POINTER)
}

#[no_mangle]
pub extern "C" fn __ctype_toupper_loc() -> *mut *const i32 {
    initialize_case_table(&UPPER_STATE, true);
    core::ptr::addr_of_mut!(UPPER_POINTER)
}

#[no_mangle]
pub extern "C" fn __ctype_get_mb_cur_max() -> usize {
    4
}

unsafe fn utf8_length(lead: u8) -> usize {
    if lead < 0x80 {
        1
    } else if lead & 0xe0 == 0xc0 {
        2
    } else if lead & 0xf0 == 0xe0 {
        3
    } else if lead & 0xf8 == 0xf0 {
        4
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn mbrtowc(
    output: *mut i32,
    input: *const c_char,
    length: usize,
    _state: *mut c_void,
) -> usize {
    if input.is_null() {
        return 0;
    }
    if length == 0 {
        return usize::MAX - 1;
    }
    let bytes = input.cast::<u8>();
    let lead = unsafe { read_volatile(bytes) };
    if lead == 0 {
        if !output.is_null() {
            unsafe { output.write(0) };
        }
        return 0;
    }
    let expected = unsafe { utf8_length(lead) };
    if expected == 0 {
        return usize::MAX;
    }
    if length < expected {
        return usize::MAX - 1;
    }
    let mut codepoint = match expected {
        1 => lead as u32,
        2 => (lead & 0x1f) as u32,
        3 => (lead & 0x0f) as u32,
        _ => (lead & 0x07) as u32,
    };
    for index in 1..expected {
        let continuation = unsafe { read_volatile(bytes.add(index)) };
        if continuation & 0xc0 != 0x80 {
            return usize::MAX;
        }
        codepoint = (codepoint << 6) | (continuation & 0x3f) as u32;
    }
    if (expected == 2 && codepoint < 0x80)
        || (expected == 3 && codepoint < 0x800)
        || (expected == 4 && codepoint < 0x10000)
        || codepoint > 0x10ffff
        || (0xd800..=0xdfff).contains(&codepoint)
    {
        return usize::MAX;
    }
    if !output.is_null() {
        unsafe { output.write(codepoint as i32) };
    }
    expected
}

#[no_mangle]
pub unsafe extern "C" fn mbrlen(input: *const c_char, length: usize, state: *mut c_void) -> usize {
    unsafe { mbrtowc(core::ptr::null_mut(), input, length, state) }
}

#[no_mangle]
pub unsafe extern "C" fn mbtowc(output: *mut i32, input: *const c_char, length: usize) -> i32 {
    let result = unsafe { mbrtowc(output, input, length, core::ptr::null_mut()) };
    if result == usize::MAX || result == usize::MAX - 1 {
        -1
    } else {
        result as i32
    }
}

#[no_mangle]
pub unsafe extern "C" fn mblen(input: *const c_char, length: usize) -> i32 {
    unsafe { mbtowc(core::ptr::null_mut(), input, length) }
}

#[no_mangle]
pub unsafe extern "C" fn wcrtomb(output: *mut c_char, input: i32, _state: *mut c_void) -> usize {
    if output.is_null() {
        return 1;
    }
    let codepoint = input as u32;
    if codepoint > 0x10ffff || (0xd800..=0xdfff).contains(&codepoint) {
        return usize::MAX;
    }
    let bytes = output.cast::<u8>();
    if codepoint < 0x80 {
        unsafe { write_volatile(bytes, codepoint as u8) };
        1
    } else if codepoint < 0x800 {
        unsafe {
            write_volatile(bytes, (0xc0 | (codepoint >> 6)) as u8);
            write_volatile(bytes.add(1), (0x80 | (codepoint & 0x3f)) as u8);
        }
        2
    } else if codepoint < 0x10000 {
        unsafe {
            write_volatile(bytes, (0xe0 | (codepoint >> 12)) as u8);
            write_volatile(bytes.add(1), (0x80 | ((codepoint >> 6) & 0x3f)) as u8);
            write_volatile(bytes.add(2), (0x80 | (codepoint & 0x3f)) as u8);
        }
        3
    } else {
        unsafe {
            write_volatile(bytes, (0xf0 | (codepoint >> 18)) as u8);
            write_volatile(bytes.add(1), (0x80 | ((codepoint >> 12) & 0x3f)) as u8);
            write_volatile(bytes.add(2), (0x80 | ((codepoint >> 6) & 0x3f)) as u8);
            write_volatile(bytes.add(3), (0x80 | (codepoint & 0x3f)) as u8);
        }
        4
    }
}

#[no_mangle]
pub unsafe extern "C" fn mbsrtowcs(
    output: *mut i32,
    input: *mut *const c_char,
    capacity: usize,
    state: *mut c_void,
) -> usize {
    if input.is_null() {
        return 0;
    }
    let mut cursor = unsafe { input.read() };
    if cursor.is_null() {
        return 0;
    }
    let mut converted = 0;
    while unsafe { read_volatile(cursor.cast::<u8>()) } != 0 {
        let mut wide = 0;
        let amount = unsafe { mbrtowc(&mut wide, cursor, 4, state) };
        if amount == usize::MAX || amount == usize::MAX - 1 {
            return usize::MAX;
        }
        if !output.is_null() {
            if converted == capacity {
                unsafe { input.write(cursor) };
                return converted;
            }
            unsafe { output.add(converted).write(wide) };
        }
        cursor = unsafe { cursor.add(amount) };
        converted += 1;
    }
    if !output.is_null() {
        if converted < capacity {
            unsafe { output.add(converted).write(0) };
        }
        unsafe { input.write(core::ptr::null()) };
    }
    converted
}

#[no_mangle]
pub unsafe extern "C" fn mbsnrtowcs(
    output: *mut i32,
    input: *mut *const c_char,
    byte_count: usize,
    capacity: usize,
    state: *mut c_void,
) -> usize {
    if output.is_null() {
        return unsafe { mbsrtowcs(output, input, capacity, state) };
    }
    if input.is_null() {
        return 0;
    }
    let mut cursor = unsafe { input.read() };
    if cursor.is_null() {
        return 0;
    }
    let mut consumed = 0;
    let mut converted = 0;
    while consumed < byte_count
        && converted < capacity
        && unsafe { read_volatile(cursor.cast::<u8>()) } != 0
    {
        let mut wide = 0;
        let amount = unsafe { mbrtowc(&mut wide, cursor, byte_count - consumed, state) };
        if amount == usize::MAX || amount == usize::MAX - 1 {
            return usize::MAX;
        }
        unsafe { output.add(converted).write(wide) };
        cursor = unsafe { cursor.add(amount) };
        consumed += amount;
        converted += 1;
    }
    if unsafe { read_volatile(cursor.cast::<u8>()) } == 0 {
        unsafe { input.write(core::ptr::null()) };
        if converted < capacity {
            unsafe { output.add(converted).write(0) };
        }
    } else {
        unsafe { input.write(cursor) };
    }
    converted
}

#[no_mangle]
pub unsafe extern "C" fn wcsnrtombs(
    output: *mut c_char,
    input: *mut *const i32,
    byte_capacity: usize,
    character_count: usize,
    state: *mut c_void,
) -> usize {
    if input.is_null() {
        return 0;
    }
    let mut cursor = unsafe { input.read() };
    if cursor.is_null() {
        return 0;
    }
    let mut bytes_written = 0;
    let mut characters = 0;
    while characters < character_count && unsafe { read_volatile(cursor) } != 0 {
        let mut encoded = [0 as c_char; 4];
        let amount = unsafe { wcrtomb(encoded.as_mut_ptr(), read_volatile(cursor), state) };
        if amount == usize::MAX {
            return usize::MAX;
        }
        if !output.is_null() && amount > byte_capacity.saturating_sub(bytes_written) {
            unsafe { input.write(cursor) };
            return bytes_written;
        }
        if !output.is_null() {
            for index in 0..amount {
                unsafe { output.add(bytes_written + index).write(encoded[index]) };
            }
        }
        bytes_written += amount;
        cursor = unsafe { cursor.add(1) };
        characters += 1;
    }
    if !output.is_null() {
        if unsafe { read_volatile(cursor) } == 0 {
            unsafe { input.write(core::ptr::null()) };
        } else {
            unsafe { input.write(cursor) };
        }
    }
    bytes_written
}

#[no_mangle]
pub extern "C" fn getwc(_stream: *mut c_void) -> u32 {
    WEOF
}

#[no_mangle]
pub extern "C" fn ungetwc(_character: i32, _stream: *mut c_void) -> u32 {
    WEOF
}

#[no_mangle]
pub unsafe extern "C" fn fputwc(character: i32, stream: *mut c_void) -> u32 {
    let mut encoded = [0 as c_char; 4];
    let length = unsafe { wcrtomb(encoded.as_mut_ptr(), character, core::ptr::null_mut()) };
    if length == usize::MAX
        || unsafe { fwrite(encoded.as_ptr().cast(), 1, length, stream) } != length
    {
        return WEOF;
    }
    character as u32
}

#[no_mangle]
pub unsafe extern "C" fn strtod_l(
    input: *const c_char,
    end: *mut *mut c_char,
    _locale: *mut c_void,
) -> f64 {
    unsafe { super::posix::strtod(input, end) }
}

#[no_mangle]
pub unsafe extern "C" fn strtof_l(
    input: *const c_char,
    end: *mut *mut c_char,
    _locale: *mut c_void,
) -> f32 {
    unsafe { super::posix::strtof(input, end) }
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_strtold_l(
    input: *const c_char,
    end: *mut *mut c_char,
    _locale: *mut c_void,
) -> f64 {
    unsafe { super::posix::strtod(input, end) }
}

#[no_mangle]
pub unsafe extern "C" fn __mbrlen(
    text: *const c_char,
    length: usize,
    _state: *mut c_void,
) -> usize {
    if text.is_null() {
        return 0;
    }
    if length == 0 {
        return usize::MAX - 1;
    }
    let lead = unsafe { read_volatile(text.cast::<u8>()) };
    if lead < 0x80 {
        return 1;
    }
    let expected = unsafe { utf8_length(lead) };
    if expected == 0 {
        return usize::MAX;
    }
    if length < expected {
        return usize::MAX - 1;
    }
    for index in 1..expected {
        if unsafe { read_volatile(text.cast::<u8>().add(index)) } & 0xc0 != 0x80 {
            return usize::MAX;
        }
    }
    expected
}

#[no_mangle]
pub unsafe extern "C" fn wmemchr(memory: *const i32, value: i32, count: usize) -> *mut i32 {
    if memory.is_null() {
        return core::ptr::null_mut();
    }
    for index in 0..count {
        if unsafe { read_volatile(memory.add(index)) } == value {
            return unsafe { memory.add(index).cast_mut() };
        }
    }
    core::ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn catopen(_name: *const c_char, _flag: i32) -> i32 {
    -1
}

#[no_mangle]
pub extern "C" fn catgets(
    _catalog: i32,
    _set_number: i32,
    _message: i32,
    text: *const c_char,
) -> *mut c_char {
    text.cast_mut()
}

#[no_mangle]
pub extern "C" fn catclose(_catalog: i32) -> i32 {
    0
}

#[no_mangle]
pub unsafe extern "C" fn strftime_l(
    output: *mut c_char,
    capacity: usize,
    _format: *const c_char,
    _time: *const c_void,
    _locale: *mut c_void,
) -> usize {
    if capacity != 0 && !output.is_null() {
        unsafe { output.write(0) };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_sscanf(
    _input: *const c_char,
    _format: *const c_char,
    _arguments: *mut c_void,
) -> i32 {
    0
}

#[no_mangle]
pub extern "C" fn abort() -> ! {
    trap()
}

#[no_mangle]
pub extern "C" fn __stack_chk_fail() -> ! {
    trap()
}

#[no_mangle]
pub static mut __libc_stack_end: usize = 0;

unsafe extern "C" {
    fn fwrite(buffer: *const c_void, size: usize, count: usize, stream: *mut c_void) -> usize;
}

#[cold]
fn trap() -> ! {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("ud2", options(noreturn));
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("brk #0", options(noreturn));
    }
}
