/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_void};
use core::ptr::{read_volatile, write_volatile};

use super::stdio::KzFile;
#[allow(unused_imports)]
use super::variadic::{
    self, VA_INT, VA_LLONG, VA_POINTER, VA_PTRDIFF, VA_SIZE, VA_ULLONG, VA_ULONG,
};

const INT_MAX: usize = i32::MAX as usize;
const EOF: i32 = -1;

struct FormatOutput {
    buffer: *mut u8,
    capacity: usize,
    length: usize,
}

unsafe extern "C" {
    fn kz_host_console_write(data: *const u8, length: usize) -> i32;
    fn malloc(size: usize) -> *mut c_void;
    fn free(pointer: *mut c_void);
}

#[repr(C, align(16))]
struct VaStorage {
    words: [usize; 8],
}

unsafe fn emit_character(output: &mut FormatOutput, character: u8) {
    if output.capacity != 0 && output.length < output.capacity - 1 {
        unsafe { write_volatile(output.buffer.add(output.length), character) };
    }
    if output.length < INT_MAX {
        output.length += 1;
    }
}

unsafe fn emit_repeat(output: &mut FormatOutput, character: u8, count: usize) {
    for _ in 0..count {
        unsafe { emit_character(output, character) };
    }
}

unsafe fn emit_text(output: &mut FormatOutput, text: &[u8]) {
    for character in text.iter().copied() {
        unsafe { emit_character(output, character) };
    }
}

fn unsigned_digits(mut value: u64, base: u64, uppercase: bool, output: &mut [u8; 64]) -> usize {
    let digits = if uppercase {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    let mut count = 0;
    loop {
        output[count] = digits[(value % base) as usize];
        count += 1;
        value /= base;
        if value == 0 {
            break;
        }
    }
    count
}

unsafe fn emit_number(
    output: &mut FormatOutput,
    value: u64,
    negative: bool,
    base: u64,
    uppercase: bool,
    alternate: bool,
    left: bool,
    zero: bool,
    plus: bool,
    space: bool,
    precision: i32,
    width: i32,
    pointer: bool,
) {
    let mut reversed = [0u8; 64];
    let mut digit_count = if value == 0 && precision == 0 {
        0
    } else {
        unsigned_digits(value, base, uppercase, &mut reversed)
    };
    let precision_zeroes = if precision > 0 && precision as usize > digit_count {
        precision as usize - digit_count
    } else {
        0
    };
    let sign = if negative {
        b'-'
    } else if plus {
        b'+'
    } else if space {
        b' '
    } else {
        0
    };
    let mut prefix = [0u8; 2];
    let mut prefix_length = 0;
    if pointer || (alternate && base == 16 && value != 0) {
        prefix[0] = b'0';
        prefix[1] = if uppercase { b'X' } else { b'x' };
        prefix_length = 2;
    } else if alternate && base == 8 && (digit_count == 0 || reversed[digit_count - 1] != b'0') {
        prefix[0] = b'0';
        prefix_length = 1;
    }
    let content_length =
        digit_count + precision_zeroes + prefix_length + if sign != 0 { 1 } else { 0 };
    let padding = if width > 0 && width as usize > content_length {
        width as usize - content_length
    } else {
        0
    };
    if !left && (!zero || precision >= 0) {
        unsafe { emit_repeat(output, b' ', padding) };
    }
    if sign != 0 {
        unsafe { emit_character(output, sign) };
    }
    unsafe { emit_text(output, &prefix[..prefix_length]) };
    if !left && zero && precision < 0 {
        unsafe { emit_repeat(output, b'0', padding) };
    }
    unsafe { emit_repeat(output, b'0', precision_zeroes) };
    while digit_count != 0 {
        digit_count -= 1;
        unsafe { emit_character(output, reversed[digit_count]) };
    }
    if left {
        unsafe { emit_repeat(output, b' ', padding) };
    }
}

unsafe fn next_argument(arguments: *mut c_void, kind: c_int) -> usize {
    unsafe { variadic::next(arguments, kind) }
}

unsafe fn format_output(
    output: &mut FormatOutput,
    format: *const c_char,
    arguments: *mut c_void,
) -> i32 {
    if format.is_null() || (output.capacity != 0 && output.buffer.is_null()) {
        return -1;
    }
    let mut cursor = format.cast::<u8>();
    loop {
        let character = unsafe { read_volatile(cursor) };
        if character == 0 {
            break;
        }
        if character != b'%' {
            unsafe { emit_character(output, character) };
            cursor = unsafe { cursor.add(1) };
            continue;
        }
        cursor = unsafe { cursor.add(1) };
        if unsafe { read_volatile(cursor) } == b'%' {
            unsafe { emit_character(output, b'%') };
            cursor = unsafe { cursor.add(1) };
            continue;
        }

        let mut left = false;
        let mut zero = false;
        let mut plus = false;
        let mut space = false;
        let mut alternate = false;
        loop {
            match unsafe { read_volatile(cursor) } {
                b'-' => left = true,
                b'0' => zero = true,
                b'+' => plus = true,
                b' ' => space = true,
                b'#' => alternate = true,
                _ => break,
            }
            cursor = unsafe { cursor.add(1) };
        }

        let mut width = 0i32;
        if unsafe { read_volatile(cursor) } == b'*' {
            width = unsafe { next_argument(arguments, VA_INT) } as i32;
            cursor = unsafe { cursor.add(1) };
            if width < 0 {
                left = true;
                width = width.saturating_abs();
            }
        } else {
            while unsafe { read_volatile(cursor) }.is_ascii_digit() {
                let digit = (unsafe { read_volatile(cursor) } - b'0') as i32;
                width = width.saturating_mul(10).saturating_add(digit);
                cursor = unsafe { cursor.add(1) };
            }
        }

        let mut precision = -1i32;
        if unsafe { read_volatile(cursor) } == b'.' {
            cursor = unsafe { cursor.add(1) };
            precision = 0;
            if unsafe { read_volatile(cursor) } == b'*' {
                precision = unsafe { next_argument(arguments, VA_INT) } as i32;
                cursor = unsafe { cursor.add(1) };
                if precision < 0 {
                    precision = -1;
                }
            } else {
                while unsafe { read_volatile(cursor) }.is_ascii_digit() {
                    let digit = (unsafe { read_volatile(cursor) } - b'0') as i32;
                    precision = precision.saturating_mul(10).saturating_add(digit);
                    cursor = unsafe { cursor.add(1) };
                }
            }
        }

        const LENGTH_DEFAULT: u8 = 0;
        const LENGTH_LONG: u8 = 1;
        const LENGTH_LONG_LONG: u8 = 2;
        const LENGTH_SIZE: u8 = 3;
        let mut length = LENGTH_DEFAULT;
        match unsafe { read_volatile(cursor) } {
            b'h' => {
                cursor = unsafe { cursor.add(1) };
                if unsafe { read_volatile(cursor) } == b'h' {
                    cursor = unsafe { cursor.add(1) };
                }
            }
            b'l' => {
                cursor = unsafe { cursor.add(1) };
                length = LENGTH_LONG;
                if unsafe { read_volatile(cursor) } == b'l' {
                    cursor = unsafe { cursor.add(1) };
                    length = LENGTH_LONG_LONG;
                }
            }
            b'z' | b't' => {
                cursor = unsafe { cursor.add(1) };
                length = LENGTH_SIZE;
            }
            _ => {}
        }

        let specifier = unsafe { read_volatile(cursor) };
        if specifier != 0 {
            cursor = unsafe { cursor.add(1) };
        }
        match specifier {
            b's' => {
                let text = unsafe { next_argument(arguments, VA_POINTER) } as *const u8;
                let default_text = b"(null)";
                let (text, mut text_length) = if text.is_null() {
                    (default_text.as_ptr(), default_text.len())
                } else {
                    let mut text_length = 0;
                    while unsafe { read_volatile(text.add(text_length)) } != 0 {
                        text_length += 1;
                    }
                    (text, text_length)
                };
                if precision >= 0 {
                    text_length = text_length.min(precision as usize);
                }
                let padding = (width as usize).saturating_sub(text_length);
                if !left {
                    unsafe { emit_repeat(output, b' ', padding) };
                }
                for index in 0..text_length {
                    unsafe { emit_character(output, read_volatile(text.add(index))) };
                }
                if left {
                    unsafe { emit_repeat(output, b' ', padding) };
                }
            }
            b'c' => {
                let character = unsafe { next_argument(arguments, VA_INT) } as u8;
                let padding = (width as usize).saturating_sub(1);
                if !left {
                    unsafe { emit_repeat(output, b' ', padding) };
                }
                unsafe { emit_character(output, character) };
                if left {
                    unsafe { emit_repeat(output, b' ', padding) };
                }
            }
            b'd' | b'i' => {
                let raw = match length {
                    LENGTH_LONG_LONG => (unsafe { next_argument(arguments, VA_LLONG) }) as i64,
                    LENGTH_LONG => {
                        (unsafe { next_argument(arguments, super::variadic::VA_LONG) }) as isize
                            as i64
                    }
                    LENGTH_SIZE => {
                        (unsafe { next_argument(arguments, VA_PTRDIFF) }) as isize as i64
                    }
                    _ => (unsafe { next_argument(arguments, VA_INT) }) as i32 as i64,
                };
                let negative = raw < 0;
                let magnitude = if negative {
                    (-(raw + 1)) as u64 + 1
                } else {
                    raw as u64
                };
                unsafe {
                    emit_number(
                        output, magnitude, negative, 10, false, false, left, zero, plus, space,
                        precision, width, false,
                    );
                }
            }
            b'u' | b'o' | b'x' | b'X' | b'p' => {
                let value = if specifier == b'p' {
                    (unsafe { next_argument(arguments, VA_POINTER) }) as u64
                } else {
                    match length {
                        LENGTH_LONG_LONG => (unsafe { next_argument(arguments, VA_ULLONG) }) as u64,
                        LENGTH_LONG => {
                            (unsafe { next_argument(arguments, super::variadic::VA_ULONG) }) as u64
                        }
                        LENGTH_SIZE => (unsafe { next_argument(arguments, VA_SIZE) }) as u64,
                        _ => {
                            (unsafe { next_argument(arguments, super::variadic::VA_UINT) }) as u32
                                as u64
                        }
                    }
                };
                let base = if specifier == b'o' {
                    8
                } else if specifier == b'u' {
                    10
                } else {
                    16
                };
                unsafe {
                    emit_number(
                        output,
                        value,
                        false,
                        base,
                        specifier == b'X',
                        alternate,
                        left,
                        zero,
                        false,
                        false,
                        precision,
                        width,
                        specifier == b'p',
                    );
                }
            }
            0 => unsafe { emit_character(output, b'%') },
            _ => unsafe {
                emit_character(output, b'%');
                emit_character(output, specifier);
            },
        }
    }
    if output.capacity != 0 {
        let terminator = output.length.min(output.capacity - 1);
        unsafe { write_volatile(output.buffer.add(terminator), 0) };
    }
    if output.length > INT_MAX {
        -1
    } else {
        output.length as i32
    }
}

unsafe fn format_from_list(
    buffer: *mut c_char,
    size: usize,
    format: *const c_char,
    arguments: *mut c_void,
) -> i32 {
    let mut storage = VaStorage { words: [0; 8] };
    let storage_pointer = (&mut storage as *mut VaStorage).cast::<c_void>();
    unsafe { variadic::copy(storage_pointer, arguments) };
    let mut output = FormatOutput {
        buffer: buffer.cast(),
        capacity: size,
        length: 0,
    };
    let result = unsafe { format_output(&mut output, format, storage_pointer) };
    unsafe { variadic::end(storage_pointer) };
    result
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_vsnprintf(
    buffer: *mut c_char,
    size: usize,
    format: *const c_char,
    arguments: *mut c_void,
) -> i32 {
    unsafe { format_from_list(buffer, size, format, arguments) }
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_vsnprintf_chk(
    buffer: *mut c_char,
    size: usize,
    _flag: i32,
    object_size: usize,
    format: *const c_char,
    arguments: *mut c_void,
) -> i32 {
    if size > object_size {
        return -1;
    }
    unsafe { format_from_list(buffer, size, format, arguments) }
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_printf(format: *const c_char, arguments: *mut c_void) -> i32 {
    let length = unsafe { format_from_list(core::ptr::null_mut(), 0, format, arguments) };
    if length < 0 {
        return EOF;
    }
    let Some(capacity) = (length as usize).checked_add(1) else {
        return EOF;
    };
    let mut buffer = Vec::new();
    if buffer.try_reserve_exact(capacity).is_err() {
        super::posix::set_errno(12);
        return EOF;
    }
    buffer.resize(capacity, 0);
    let written =
        unsafe { format_from_list(buffer.as_mut_ptr().cast(), capacity, format, arguments) };
    if written != length {
        return EOF;
    }
    for chunk in buffer[..length as usize].chunks(512) {
        let result = unsafe { kz_host_console_write(chunk.as_ptr(), chunk.len()) };
        if result != chunk.len() as i32 {
            return EOF;
        }
    }
    length
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_vfprintf(
    stream: *mut KzFile,
    format: *const c_char,
    arguments: *mut c_void,
) -> i32 {
    if stream.is_null() {
        return EOF;
    }
    let mut buffer = [0u8; 1024];
    let length =
        unsafe { format_from_list(buffer.as_mut_ptr().cast(), buffer.len(), format, arguments) };
    if length < 0 {
        return -1;
    }
    let count = (length as usize).min(buffer.len() - 1);
    let result = unsafe { fwrite(buffer.as_ptr().cast(), 1, count, stream) };
    if result == count {
        result as i32
    } else {
        -1
    }
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_vprintf(format: *const c_char, arguments: *mut c_void) -> i32 {
    let stdout = unsafe { super::stdio::stdout };
    unsafe { kz_rust_vfprintf(stdout, format, arguments) }
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_vasprintf(
    output: *mut *mut c_char,
    format: *const c_char,
    arguments: *mut c_void,
) -> i32 {
    if output.is_null() {
        return -1;
    }
    unsafe { output.write(core::ptr::null_mut()) };
    let length = unsafe { format_from_list(core::ptr::null_mut(), 0, format, arguments) };
    if length < 0 {
        return -1;
    }
    let Some(capacity) = (length as usize).checked_add(1) else {
        return -1;
    };
    let buffer = unsafe { malloc(capacity) }.cast::<c_char>();
    if buffer.is_null() {
        return -1;
    }
    if unsafe { format_from_list(buffer, capacity, format, arguments) } != length {
        unsafe { free(buffer.cast()) };
        return -1;
    }
    unsafe { output.write(buffer) };
    length
}

#[no_mangle]
pub unsafe extern "C" fn fwrite(
    buffer: *const c_void,
    size: usize,
    count: usize,
    stream: *mut KzFile,
) -> usize {
    if stream.is_null() || (buffer.is_null() && size != 0 && count != 0) {
        return 0;
    }
    if size != 0 && count > usize::MAX / size {
        return 0;
    }
    let bytes = size * count;
    if size == 0 {
        return 0;
    }
    let file_descriptor = unsafe { (*stream).descriptor };
    let written = unsafe { super::filesystem::write(file_descriptor, buffer, bytes) };
    if written < 0 {
        0
    } else {
        written as usize / size
    }
}
