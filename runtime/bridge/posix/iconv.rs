use core::ffi::{c_char, c_int, c_void};
use core::mem::size_of;
use core::ptr::{addr_of, read_volatile};
use super::errno::{E2BIG, EBADF, EILSEQ, EINVAL};

#[repr(usize)]
#[derive(Clone, Copy, PartialEq)]
enum Charset {
    Ascii = 0,
    Latin1 = 1,
    Utf8 = 2,
}

#[repr(C)]
struct Converter {
    from: Charset,
    to: Charset,
}

static CONVERTERS: [[Converter; 3]; 3] = [
    [
        Converter { from: Charset::Ascii, to: Charset::Ascii },
        Converter { from: Charset::Ascii, to: Charset::Latin1 },
        Converter { from: Charset::Ascii, to: Charset::Utf8 },
    ],
    [
        Converter { from: Charset::Latin1, to: Charset::Ascii },
        Converter { from: Charset::Latin1, to: Charset::Latin1 },
        Converter { from: Charset::Latin1, to: Charset::Utf8 },
    ],
    [
        Converter { from: Charset::Utf8, to: Charset::Ascii },
        Converter { from: Charset::Utf8, to: Charset::Latin1 },
        Converter { from: Charset::Utf8, to: Charset::Utf8 },
    ],
];

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

unsafe fn parse_charset(name: *const c_char) -> Option<Charset> {
    if name.is_null() {
        return None;
    }
    let mut normalized = [0; 16];
    let mut length = 0;
    for index in 0..32 {
        let value = unsafe { read_volatile(name.cast::<u8>().add(index)) };
        if value == 0 {
            break;
        }
        if matches!(value, b'-' | b'_' | b'.') {
            continue;
        }
        if length == normalized.len() {
            return None;
        }
        normalized[length] = value.to_ascii_uppercase();
        length += 1;
        if index == 31 {
            return None;
        }
    }
    match &normalized[..length] {
        b"ASCII" | b"USASCII" => Some(Charset::Ascii),
        b"LATIN1" | b"ISO88591" => Some(Charset::Latin1),
        b"UTF8" => Some(Charset::Utf8),
        _ => None,
    }
}

fn converter_pointer(from: Charset, to: Charset) -> *mut c_void {
    addr_of!(CONVERTERS[from as usize][to as usize])
        .cast_mut()
        .cast()
}

fn is_converter(pointer: *mut c_void) -> bool {
    let start = addr_of!(CONVERTERS).cast::<Converter>() as usize;
    let end = start + size_of::<[[Converter; 3]; 3]>();
    let address = pointer as usize;
    address >= start
        && address < end
        && (address - start).is_multiple_of(size_of::<Converter>())
}

#[no_mangle]
pub unsafe extern "C" fn iconv_open(
    to_code: *const c_char,
    from_code: *const c_char,
) -> *mut c_void {
    let Some(to) = (unsafe { parse_charset(to_code) }) else {
        set_errno(EINVAL);
        return usize::MAX as *mut c_void;
    };
    let Some(from) = (unsafe { parse_charset(from_code) }) else {
        set_errno(EINVAL);
        return usize::MAX as *mut c_void;
    };
    converter_pointer(from, to)
}

#[no_mangle]
pub unsafe extern "C" fn iconv_close(conversion: *mut c_void) -> c_int {
    if is_converter(conversion) {
        0
    } else {
        set_errno(EBADF);
        -1
    }
}

unsafe fn decode_utf8(input: *const u8, input_left: usize) -> Result<(u32, usize), c_int> {
    let first = unsafe { read_volatile(input) };
    if first < 0x80 {
        return Ok((first as u32, 1));
    }
    let (length, mut codepoint) = match first {
        0xc2..=0xdf => (2, (first & 0x1f) as u32),
        0xe0..=0xef => (3, (first & 0x0f) as u32),
        0xf0..=0xf4 => (4, (first & 0x07) as u32),
        _ => return Err(EILSEQ),
    };
    if input_left < length {
        return Err(EINVAL);
    }
    for index in 1..length {
        let next = unsafe { read_volatile(input.add(index)) };
        if next & 0xc0 != 0x80 {
            return Err(EILSEQ);
        }
        if index == 1 {
            match (first, next) {
                (0xe0, 0x80..=0x9f) | (0xed, 0xa0..=0xbf) => return Err(EILSEQ),
                (0xf0, 0x80..=0x8f) | (0xf4, 0x90..=0xbf) => return Err(EILSEQ),
                _ => {}
            }
        }
        codepoint = (codepoint << 6) | (next & 0x3f) as u32;
    }
    Ok((codepoint, length))
}

unsafe fn decode(charset: Charset, input: *const u8, input_left: usize) -> Result<(u32, usize), c_int> {
    match charset {
        Charset::Ascii => {
            let byte = unsafe { read_volatile(input) };
            if byte > 0x7f {
                Err(EILSEQ)
            } else {
                Ok((byte as u32, 1))
            }
        }
        Charset::Latin1 => Ok((unsafe { read_volatile(input) } as u32, 1)),
        Charset::Utf8 => unsafe { decode_utf8(input, input_left) },
    }
}

fn encode(charset: Charset, codepoint: u32) -> Result<([u8; 4], usize), c_int> {
    let mut output = [0; 4];
    match charset {
        Charset::Ascii if codepoint <= 0x7f => {
            output[0] = codepoint as u8;
            Ok((output, 1))
        }
        Charset::Latin1 if codepoint <= 0xff => {
            output[0] = codepoint as u8;
            Ok((output, 1))
        }
        Charset::Ascii | Charset::Latin1 => Err(EILSEQ),
        Charset::Utf8 if codepoint <= 0x7f => {
            output[0] = codepoint as u8;
            Ok((output, 1))
        }
        Charset::Utf8 if codepoint <= 0x7ff => {
            output[0] = 0xc0 | (codepoint >> 6) as u8;
            output[1] = 0x80 | (codepoint & 0x3f) as u8;
            Ok((output, 2))
        }
        Charset::Utf8 if codepoint <= 0xffff => {
            output[0] = 0xe0 | (codepoint >> 12) as u8;
            output[1] = 0x80 | ((codepoint >> 6) & 0x3f) as u8;
            output[2] = 0x80 | (codepoint & 0x3f) as u8;
            Ok((output, 3))
        }
        Charset::Utf8 if codepoint <= 0x10ffff => {
            output[0] = 0xf0 | (codepoint >> 18) as u8;
            output[1] = 0x80 | ((codepoint >> 12) & 0x3f) as u8;
            output[2] = 0x80 | ((codepoint >> 6) & 0x3f) as u8;
            output[3] = 0x80 | (codepoint & 0x3f) as u8;
            Ok((output, 4))
        }
        Charset::Utf8 => Err(EILSEQ),
    }
}

#[no_mangle]
pub unsafe extern "C" fn iconv(
    conversion: *mut c_void,
    input: *mut *mut c_char,
    input_left: *mut usize,
    output: *mut *mut c_char,
    output_left: *mut usize,
) -> usize {
    if input.is_null() || unsafe { (*input).is_null() } {
        return 0;
    }
    if input_left.is_null() || output.is_null() || output_left.is_null()
        || unsafe { (*output).is_null() }
    {
        set_errno(EINVAL);
        return usize::MAX;
    }
    if !is_converter(conversion) {
        set_errno(EBADF);
        return usize::MAX;
    }

    let converter = unsafe { &*conversion.cast::<Converter>() };
    let mut input_pointer = unsafe { *input }.cast::<u8>();
    let mut input_remaining = unsafe { *input_left };
    let mut output_pointer = unsafe { *output }.cast::<u8>();
    let mut output_remaining = unsafe { *output_left };

    while input_remaining != 0 {
        let (codepoint, consumed) = match unsafe {
            decode(converter.from, input_pointer, input_remaining)
        } {
            Ok(decoded) => decoded,
            Err(error) => {
                unsafe {
                    input.write(input_pointer.cast());
                    input_left.write(input_remaining);
                    output.write(output_pointer.cast());
                    output_left.write(output_remaining);
                }
                set_errno(error);
                return usize::MAX;
            }
        };
        let (encoded, produced) = match encode(converter.to, codepoint) {
            Ok(encoded) => encoded,
            Err(error) => {
                unsafe {
                    input.write(input_pointer.cast());
                    input_left.write(input_remaining);
                    output.write(output_pointer.cast());
                    output_left.write(output_remaining);
                }
                set_errno(error);
                return usize::MAX;
            }
        };
        if output_remaining < produced {
            unsafe {
                input.write(input_pointer.cast());
                input_left.write(input_remaining);
                output.write(output_pointer.cast());
                output_left.write(output_remaining);
            }
            set_errno(E2BIG);
            return usize::MAX;
        }

        for (index, byte) in encoded[..produced].iter().copied().enumerate() {
            unsafe { output_pointer.add(index).write(byte) };
        }
        input_pointer = unsafe { input_pointer.add(consumed) };
        input_remaining -= consumed;
        output_pointer = unsafe { output_pointer.add(produced) };
        output_remaining -= produced;
    }

    unsafe {
        input.write(input_pointer.cast());
        input_left.write(input_remaining);
        output.write(output_pointer.cast());
        output_left.write(output_remaining);
    }
    0
}