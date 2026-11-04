use core::ffi::{c_char, c_int, c_long, c_void};
use super::errno::{EINVAL, ENOSYS};

const EOF: c_int = -1;
const SEEK_SET: c_int = 0;

#[repr(C)]
struct KzFile {
    descriptor: c_int,
    pushback: c_int,
    has_pushback: c_int,
}

unsafe extern "C" {
    fn fgetc(stream: *mut KzFile) -> c_int;
    fn getc(stream: *mut KzFile) -> c_int;
    fn getchar() -> c_int;
    fn fputc(character: c_int, stream: *mut KzFile) -> c_int;
    fn putchar(character: c_int) -> c_int;
    fn fflush(stream: *mut KzFile) -> c_int;
    fn fseek(stream: *mut KzFile, offset: isize, origin: c_int) -> c_int;
    fn fseeko64(stream: *mut KzFile, offset: i64, origin: c_int) -> c_int;
    fn ftello64(stream: *mut KzFile) -> i64;
    fn fileno(stream: *mut KzFile) -> c_int;
    fn unlink(path: *const c_char) -> c_int;
    fn rmdir(path: *const c_char) -> c_int;
    fn realloc(pointer: *mut c_void, size: usize) -> *mut c_void;
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub unsafe extern "C" fn clearerr(_stream: *mut c_void) {}

#[no_mangle]
pub unsafe extern "C" fn feof(_stream: *mut c_void) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn ferror(_stream: *mut c_void) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn fgetpos(stream: *mut c_void, position: *mut i64) -> c_int {
    if stream.is_null() || position.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let position_value = unsafe { ftello64(stream.cast()) };
    if position_value < 0 {
        return -1;
    }
    unsafe { position.write(position_value) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn fsetpos(stream: *mut c_void, position: *const i64) -> c_int {
    if stream.is_null() || position.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    unsafe { fseeko64(stream.cast(), position.read(), SEEK_SET) }
}

#[no_mangle]
pub unsafe extern "C" fn fseeko(stream: *mut c_void, offset: i64, origin: c_int) -> c_int {
    unsafe { fseeko64(stream.cast(), offset, origin) }
}

#[no_mangle]
pub unsafe extern "C" fn ftell(stream: *mut c_void) -> c_long {
    let position = unsafe { ftello64(stream.cast()) };
    if position > c_long::MAX as i64 || position < c_long::MIN as i64 {
        set_errno(EINVAL);
        -1
    } else {
        position as c_long
    }
}

#[no_mangle]
pub unsafe extern "C" fn ftello(stream: *mut c_void) -> i64 {
    unsafe { ftello64(stream.cast()) }
}

#[no_mangle]
pub unsafe extern "C" fn rewind(stream: *mut c_void) {
    let _ = unsafe { fseek(stream.cast(), 0, SEEK_SET) };
    unsafe { fflush(stream.cast()) };
}

#[no_mangle]
pub unsafe extern "C" fn fgets(
    buffer: *mut c_char,
    size: c_int,
    stream: *mut c_void,
) -> *mut c_char {
    if buffer.is_null() || stream.is_null() || size <= 0 {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    let mut length = 0;
    while length + 1 < size as usize {
        let character = unsafe { fgetc(stream.cast()) };
        if character == EOF {
            if length == 0 {
                return core::ptr::null_mut();
            }
            break;
        }
        unsafe { buffer.add(length).write(character as u8 as c_char) };
        length += 1;
        if character == b'\n' as c_int {
            break;
        }
    }
    unsafe { buffer.add(length).write(0) };
    buffer
}

#[no_mangle]
pub unsafe extern "C" fn getc_unlocked(stream: *mut c_void) -> c_int {
    unsafe { getc(stream.cast()) }
}

#[no_mangle]
pub unsafe extern "C" fn getchar_unlocked() -> c_int {
    unsafe { getchar() }
}

#[no_mangle]
pub unsafe extern "C" fn putc(character: c_int, stream: *mut c_void) -> c_int {
    unsafe { fputc(character, stream.cast()) }
}

#[no_mangle]
pub unsafe extern "C" fn putc_unlocked(character: c_int, stream: *mut c_void) -> c_int {
    unsafe { fputc(character, stream.cast()) }
}

#[no_mangle]
pub unsafe extern "C" fn putchar_unlocked(character: c_int) -> c_int {
    unsafe { putchar(character) }
}

#[no_mangle]
pub unsafe extern "C" fn getdelim(
    line: *mut *mut c_char,
    capacity: *mut usize,
    delimiter: c_int,
    stream: *mut c_void,
) -> isize {
    if line.is_null() || capacity.is_null() || stream.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    if unsafe { (*line).is_null() || *capacity == 0 } {
        let buffer = unsafe { realloc((*line).cast(), 128) }.cast::<c_char>();
        if buffer.is_null() {
            return -1;
        }
        unsafe {
            line.write(buffer);
            capacity.write(128);
        }
    }

    let mut length = 0usize;
    loop {
        if length + 1 >= unsafe { *capacity } {
            let Some(new_capacity) = unsafe { (*capacity).checked_mul(2) } else {
                set_errno(EINVAL);
                return -1;
            };
            let buffer = unsafe { realloc((*line).cast(), new_capacity) }.cast::<c_char>();
            if buffer.is_null() {
                return -1;
            }
            unsafe {
                line.write(buffer);
                capacity.write(new_capacity);
            }
        }
        let character = unsafe { fgetc(stream.cast()) };
        if character == EOF {
            if length == 0 {
                return -1;
            }
            break;
        }
        unsafe { (*line).add(length).write(character as u8 as c_char) };
        length += 1;
        if character == delimiter {
            break;
        }
    }
    unsafe { (*line).add(length).write(0) };
    length as isize
}

#[no_mangle]
pub unsafe extern "C" fn getline(
    line: *mut *mut c_char,
    capacity: *mut usize,
    stream: *mut c_void,
) -> isize {
    unsafe { getdelim(line, capacity, b'\n' as c_int, stream) }
}

// FIXME: The runtime has no alternate stream objects or child-process pipes.
#[no_mangle]
pub unsafe extern "C" fn fmemopen(
    _buffer: *mut c_void,
    _size: usize,
    _mode: *const c_char,
) -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn open_memstream(
    _buffer: *mut *mut c_char,
    _size: *mut usize,
) -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn freopen(
    _path: *const c_char,
    _mode: *const c_char,
    _stream: *mut c_void,
) -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn pclose(_stream: *mut c_void) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn popen(_command: *const c_char, _mode: *const c_char) -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn tmpnam(_buffer: *mut c_char) -> *mut c_char {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn remove(path: *const c_char) -> c_int {
    let result = unsafe { unlink(path) };
    if result == 0 {
        return 0;
    }
    unsafe { rmdir(path) }
}

#[no_mangle]
pub unsafe extern "C" fn rename(_old_path: *const c_char, _new_path: *const c_char) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn renameat(
    _old_directory: c_int,
    _old_path: *const c_char,
    _new_directory: c_int,
    _new_path: *const c_char,
) -> c_int {
    set_errno(ENOSYS);
    -1
}