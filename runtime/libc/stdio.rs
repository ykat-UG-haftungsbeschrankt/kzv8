/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use core::ffi::{c_char, c_void};
use core::ptr::{read_volatile, write_volatile};

const EOF: i32 = -1;
const EINVAL: i32 = 22;
const EFAULT: i32 = 14;
const EOVERFLOW: i32 = 75;
const EBADF: i32 = 9;

const O_ACCMODE: i32 = 0o3;
const O_RDONLY: i32 = 0o0;
const O_WRONLY: i32 = 0o1;
const O_RDWR: i32 = 0o2;
const O_CREAT: i32 = 0o100;
const O_EXCL: i32 = 0o200;
const O_TRUNC: i32 = 0o1000;
const O_APPEND: i32 = 0o2000;
const O_LARGEFILE: i32 = 0o100000;
const O_CLOEXEC: i32 = 0o2000000;

const IOFBF: i32 = 0;
const IOLBF: i32 = 1;
const IONBF: i32 = 2;
const OUTPUT_BUFFER_SIZE: usize = 512;

#[repr(C)]
pub(crate) struct KzFile {
    pub(crate) descriptor: i32,
    pub(crate) pushback: i32,
    pub(crate) has_pushback: i32,
}

static mut STDIN_FILE: KzFile = KzFile {
    descriptor: 0,
    pushback: 0,
    has_pushback: 0,
};
static mut STDOUT_FILE: KzFile = KzFile {
    descriptor: 1,
    pushback: 0,
    has_pushback: 0,
};
static mut STDERR_FILE: KzFile = KzFile {
    descriptor: 2,
    pushback: 0,
    has_pushback: 0,
};

#[no_mangle]
pub static mut stdin: *mut KzFile = core::ptr::addr_of_mut!(STDIN_FILE);
#[no_mangle]
pub static mut stdout: *mut KzFile = core::ptr::addr_of_mut!(STDOUT_FILE);
#[no_mangle]
pub static mut stderr: *mut KzFile = core::ptr::addr_of_mut!(STDERR_FILE);

unsafe extern "C" {
    fn kz_host_console_write(data: *const u8, length: usize) -> i32;
}

fn set_errno(error: i32) {
    super::filesystem::set_errno_for_stdio(error);
}

fn standard_stream(stream: *mut KzFile) -> bool {
    stream == core::ptr::addr_of_mut!(STDIN_FILE)
        || stream == core::ptr::addr_of_mut!(STDOUT_FILE)
        || stream == core::ptr::addr_of_mut!(STDERR_FILE)
}

unsafe fn allocate_stream(file_descriptor: i32) -> *mut KzFile {
    Box::into_raw(Box::new(KzFile {
        descriptor: file_descriptor,
        pushback: 0,
        has_pushback: 0,
    }))
}

#[no_mangle]
pub extern "C" fn putchar(character: i32) -> i32 {
    let byte = character as u8;
    let result = unsafe { kz_host_console_write(&byte, 1) };
    if result == 1 {
        byte as i32
    } else {
        EOF
    }
}

#[no_mangle]
pub unsafe extern "C" fn fclose(stream: *mut KzFile) -> i32 {
    if stream.is_null() {
        set_errno(EINVAL);
        return EOF;
    }
    if standard_stream(stream) {
        return unsafe { fflush(stream) };
    }
    let file_descriptor = unsafe { (*stream).descriptor };
    let result = super::filesystem::close(file_descriptor);
    unsafe { drop(Box::from_raw(stream)) };
    if result == 0 {
        0
    } else {
        EOF
    }
}

#[no_mangle]
pub unsafe extern "C" fn fflush(stream: *mut KzFile) -> i32 {
    if stream.is_null() {
        return 0;
    }
    let file_descriptor = unsafe { (*stream).descriptor };
    match super::filesystem::status_for_fd(file_descriptor) {
        Ok(_) => 0,
        Err(error) => {
            set_errno(error);
            EOF
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn setvbuf(
    stream: *mut KzFile,
    _buffer: *mut c_char,
    mode: i32,
    _size: usize,
) -> i32 {
    if !stream.is_null() && matches!(mode, IOFBF | IOLBF | IONBF) {
        0
    } else {
        EOF
    }
}

#[no_mangle]
pub unsafe extern "C" fn fputc(character: i32, stream: *mut KzFile) -> i32 {
    let byte = character as u8;
    if unsafe { super::format::fwrite((&byte as *const u8).cast(), 1, 1, stream) } == 1 {
        byte as i32
    } else {
        EOF
    }
}

#[no_mangle]
pub unsafe extern "C" fn fputs(text: *const c_char, stream: *mut KzFile) -> i32 {
    if text.is_null() || stream.is_null() {
        return EOF;
    }
    let bytes = text.cast::<u8>();
    let mut length = 0;
    while unsafe { read_volatile(bytes.add(length)) } != 0 {
        length += 1;
    }
    if unsafe { super::format::fwrite(text.cast(), 1, length, stream) } == length {
        0
    } else {
        EOF
    }
}

#[no_mangle]
pub unsafe extern "C" fn fileno(stream: *mut KzFile) -> i32 {
    if stream.is_null() {
        set_errno(EBADF);
        -1
    } else {
        unsafe { (*stream).descriptor }
    }
}

fn parse_file_mode(mode: *const c_char, allow_exclusive: bool) -> Result<i32, i32> {
    if mode.is_null() {
        return Err(EINVAL);
    }
    let bytes = mode.cast::<u8>();
    let initial = unsafe { read_volatile(bytes) };
    if !matches!(initial, b'r' | b'w' | b'a') {
        return Err(EINVAL);
    }
    let mut plus = false;
    let mut binary = false;
    let mut exclusive = false;
    let mut close_on_exec = false;
    let mut index = 1;
    loop {
        let character = unsafe { read_volatile(bytes.add(index)) };
        if character == 0 {
            break;
        }
        match character {
            b'+' if !plus => plus = true,
            b'b' if !binary => binary = true,
            b'x' if !exclusive && allow_exclusive && initial != b'r' => exclusive = true,
            b'e' if !close_on_exec => close_on_exec = true,
            _ => return Err(EINVAL),
        }
        index += 1;
    }
    let mut flags = O_LARGEFILE;
    match initial {
        b'r' => flags |= O_RDONLY,
        b'w' => flags |= O_WRONLY | O_CREAT | O_TRUNC,
        _ => flags |= O_WRONLY | O_CREAT | O_APPEND,
    }
    if plus {
        flags = (flags & !O_ACCMODE) | O_RDWR;
    }
    if exclusive {
        flags |= O_EXCL;
    }
    if close_on_exec {
        flags |= O_CLOEXEC;
    }
    Ok(flags)
}

#[no_mangle]
pub unsafe extern "C" fn fdopen(file_descriptor: i32, mode: *const c_char) -> *mut KzFile {
    let flags = match parse_file_mode(mode, false) {
        Ok(flags) => flags,
        Err(error) => {
            set_errno(error);
            return core::ptr::null_mut();
        }
    };
    let access = flags & O_ACCMODE;
    if file_descriptor == 0 {
        if access != O_RDONLY {
            set_errno(EBADF);
            return core::ptr::null_mut();
        }
        return unsafe { stdin };
    }
    if file_descriptor == 1 || file_descriptor == 2 {
        if access != O_WRONLY {
            set_errno(EBADF);
            return core::ptr::null_mut();
        }
        return if file_descriptor == 1 {
            unsafe { stdout }
        } else {
            unsafe { stderr }
        };
    }
    if let Err(error) =
        super::filesystem::fd_access(file_descriptor, access != O_WRONLY, access != O_RDONLY)
    {
        set_errno(error);
        return core::ptr::null_mut();
    }
    unsafe { allocate_stream(file_descriptor) }
}

#[no_mangle]
pub unsafe extern "C" fn fopen64(path: *const c_char, mode: *const c_char) -> *mut KzFile {
    let flags = match parse_file_mode(mode, true) {
        Ok(flags) => flags,
        Err(error) => {
            set_errno(error);
            return core::ptr::null_mut();
        }
    };
    let file_descriptor = unsafe { super::filesystem::kz_rust_open(path, flags, 0o666) };
    if file_descriptor < 0 {
        return core::ptr::null_mut();
    }
    unsafe { allocate_stream(file_descriptor) }
}

#[no_mangle]
pub unsafe extern "C" fn fopen(path: *const c_char, mode: *const c_char) -> *mut KzFile {
    unsafe { fopen64(path, mode) }
}

#[no_mangle]
pub unsafe extern "C" fn tmpfile64() -> *mut KzFile {
    let file_descriptor = match super::filesystem::tmpfile_descriptor() {
        Ok(file_descriptor) => file_descriptor,
        Err(error) => {
            set_errno(error);
            return core::ptr::null_mut();
        }
    };
    unsafe { allocate_stream(file_descriptor) }
}

#[no_mangle]
pub unsafe extern "C" fn tmpfile() -> *mut KzFile {
    unsafe { tmpfile64() }
}

#[no_mangle]
pub unsafe extern "C" fn fread(
    buffer: *mut c_void,
    size: usize,
    count: usize,
    stream: *mut KzFile,
) -> usize {
    if stream.is_null() {
        set_errno(EINVAL);
        return 0;
    }
    if size == 0 || count == 0 {
        return 0;
    }
    if buffer.is_null() {
        set_errno(EFAULT);
        return 0;
    }
    let Some(total) = size.checked_mul(count) else {
        set_errno(EOVERFLOW);
        return 0;
    };
    let output = buffer.cast::<u8>();
    let mut amount_read = 0;
    if unsafe { (*stream).has_pushback != 0 } {
        unsafe {
            write_volatile(output, (*stream).pushback as u8);
            (*stream).has_pushback = 0;
        }
        amount_read = 1;
    }
    let file_descriptor = unsafe { (*stream).descriptor };
    while amount_read < total {
        let request = (total - amount_read).min(isize::MAX as usize);
        let amount = unsafe {
            super::filesystem::read(file_descriptor, output.add(amount_read).cast(), request)
        };
        if amount <= 0 {
            break;
        }
        amount_read += amount as usize;
    }
    amount_read / size
}

#[no_mangle]
pub unsafe extern "C" fn fgetc(stream: *mut KzFile) -> i32 {
    if stream.is_null() {
        set_errno(EINVAL);
        return EOF;
    }
    if unsafe { (*stream).has_pushback != 0 } {
        unsafe {
            (*stream).has_pushback = 0;
            return (*stream).pushback as u8 as i32;
        }
    }
    let mut character = 0u8;
    let result = unsafe {
        super::filesystem::read((*stream).descriptor, (&mut character as *mut u8).cast(), 1)
    };
    if result == 1 {
        character as i32
    } else {
        EOF
    }
}

#[no_mangle]
pub unsafe extern "C" fn getc(stream: *mut KzFile) -> i32 {
    unsafe { fgetc(stream) }
}

#[no_mangle]
pub unsafe extern "C" fn getchar() -> i32 {
    unsafe { fgetc(core::ptr::addr_of_mut!(STDIN_FILE)) }
}

#[no_mangle]
pub unsafe extern "C" fn ungetc(character: i32, stream: *mut KzFile) -> i32 {
    if stream.is_null() || character == EOF || unsafe { (*stream).has_pushback != 0 } {
        return EOF;
    }
    unsafe {
        (*stream).pushback = character as u8 as i32;
        (*stream).has_pushback = 1;
        (*stream).pushback
    }
}

#[no_mangle]
pub unsafe extern "C" fn puts(text: *const c_char) -> i32 {
    if text.is_null() {
        return EOF;
    }
    let bytes = text.cast::<u8>();
    let mut output = [0u8; OUTPUT_BUFFER_SIZE];
    let mut length = 0usize;
    let mut total = 0usize;
    let mut index = 0usize;
    loop {
        let character = unsafe { read_volatile(bytes.add(index)) };
        index += 1;
        let character = if character == 0 { b'\n' } else { character };
        output[length] = character;
        length += 1;
        total += 1;
        if length == OUTPUT_BUFFER_SIZE || character == b'\n' {
            let result = unsafe { kz_host_console_write(output.as_ptr(), length) };
            if result != length as i32 {
                return EOF;
            }
            length = 0;
        }
        if character == b'\n' {
            break;
        }
    }
    total as i32
}

#[no_mangle]
pub unsafe extern "C" fn setbuf(stream: *mut KzFile, buffer: *mut c_char) -> i32 {
    unsafe { setvbuf(stream, buffer, IOFBF, 0) }
}

#[no_mangle]
pub unsafe extern "C" fn fseek(stream: *mut KzFile, offset: isize, origin: i32) -> i32 {
    if stream.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut offset = offset as i64;
    if unsafe { (*stream).has_pushback != 0 } && origin == 1 {
        if offset == i64::MIN {
            set_errno(EINVAL);
            return -1;
        }
        offset -= 1;
    }
    let position = super::filesystem::lseek(unsafe { (*stream).descriptor }, offset, origin);
    if position < 0 {
        return -1;
    }
    unsafe { (*stream).has_pushback = 0 };
    0
}

#[no_mangle]
pub unsafe extern "C" fn fseeko64(stream: *mut KzFile, offset: i64, origin: i32) -> i32 {
    if stream.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut offset = offset;
    if unsafe { (*stream).has_pushback != 0 } && origin == 1 {
        if offset == i64::MIN {
            set_errno(EINVAL);
            return -1;
        }
        offset -= 1;
    }
    if super::filesystem::lseek(unsafe { (*stream).descriptor }, offset, origin) < 0 {
        return -1;
    }
    unsafe { (*stream).has_pushback = 0 };
    0
}

#[no_mangle]
pub unsafe extern "C" fn ftello64(stream: *mut KzFile) -> i64 {
    if stream.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let position = super::filesystem::lseek(unsafe { (*stream).descriptor }, 0, 1);
    if position < 0 {
        return -1;
    }
    if unsafe { (*stream).has_pushback != 0 } && position > 0 {
        position - 1
    } else {
        position
    }
}
