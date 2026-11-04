use core::ffi::c_char;
use core::ptr::{read_volatile, write_volatile};

static mut DOT: [u8; 2] = [b'.', 0];

unsafe fn path_length(path: *mut u8) -> usize {
    let mut length = 0;
    while unsafe { read_volatile(path.add(length)) } != 0 {
        length += 1;
    }
    length
}

fn dot() -> *mut c_char {
    core::ptr::addr_of_mut!(DOT).cast::<u8>().cast()
}

#[no_mangle]
pub unsafe extern "C" fn basename(path: *mut c_char) -> *mut c_char {
    if path.is_null() {
        return dot();
    }
    let path = path.cast::<u8>();
    let mut length = unsafe { path_length(path) };
    if length == 0 {
        return dot();
    }
    while length > 1 && unsafe { read_volatile(path.add(length - 1)) } == b'/' {
        length -= 1;
    }
    if length == 1 && unsafe { read_volatile(path) } == b'/' {
        return path.cast();
    }
    unsafe { write_volatile(path.add(length), 0) };
    let mut start = length;
    while start != 0 && unsafe { read_volatile(path.add(start - 1)) } != b'/' {
        start -= 1;
    }
    unsafe { path.add(start).cast() }
}

#[no_mangle]
pub unsafe extern "C" fn dirname(path: *mut c_char) -> *mut c_char {
    if path.is_null() {
        return dot();
    }
    let path = path.cast::<u8>();
    let mut length = unsafe { path_length(path) };
    if length == 0 {
        return dot();
    }
    while length > 1 && unsafe { read_volatile(path.add(length - 1)) } == b'/' {
        length -= 1;
    }
    if length == 1 && unsafe { read_volatile(path) } == b'/' {
        return path.cast();
    }

    let mut slash = length;
    while slash != 0 && unsafe { read_volatile(path.add(slash - 1)) } != b'/' {
        slash -= 1;
    }
    if slash == 0 {
        return dot();
    }
    let mut end = slash - 1;
    while end != 0 && unsafe { read_volatile(path.add(end - 1)) } == b'/' {
        end -= 1;
    }
    if end == 0 {
        unsafe { write_volatile(path.add(1), 0) };
        return path.cast();
    }
    unsafe { write_volatile(path.add(end), 0) };
    path.cast()
}