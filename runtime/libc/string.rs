/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::c_void;
use core::ptr::{read_volatile, write_volatile};

#[no_mangle]
pub unsafe extern "C" fn memcpy(
    destination: *mut c_void,
    source: *const c_void,
    count: usize,
) -> *mut c_void {
    let destination = destination.cast::<u8>();
    let source = source.cast::<u8>();
    for index in 0..count {
        unsafe { write_volatile(destination.add(index), read_volatile(source.add(index))) };
    }
    destination.cast()
}

#[no_mangle]
pub unsafe extern "C" fn memmove(
    destination: *mut c_void,
    source: *const c_void,
    count: usize,
) -> *mut c_void {
    let destination_bytes = destination.cast::<u8>();
    let source_bytes = source.cast::<u8>();
    let destination_address = destination_bytes as usize;
    let source_address = source_bytes as usize;
    if destination_address <= source_address || destination_address - source_address >= count {
        for index in 0..count {
            unsafe {
                write_volatile(
                    destination_bytes.add(index),
                    read_volatile(source_bytes.add(index)),
                );
            }
        }
    } else {
        for index in (0..count).rev() {
            unsafe {
                write_volatile(
                    destination_bytes.add(index),
                    read_volatile(source_bytes.add(index)),
                );
            }
        }
    }
    destination
}

#[no_mangle]
pub unsafe extern "C" fn memset(destination: *mut c_void, value: i32, count: usize) -> *mut c_void {
    let destination = destination.cast::<u8>();
    for index in 0..count {
        unsafe { write_volatile(destination.add(index), value as u8) };
    }
    destination.cast()
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(left: *const c_void, right: *const c_void, count: usize) -> i32 {
    let left = left.cast::<u8>();
    let right = right.cast::<u8>();
    for index in 0..count {
        let left_byte = unsafe { read_volatile(left.add(index)) };
        let right_byte = unsafe { read_volatile(right.add(index)) };
        if left_byte != right_byte {
            return left_byte as i32 - right_byte as i32;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn bcmp(left: *const c_void, right: *const c_void, count: usize) -> i32 {
    unsafe { memcmp(left, right, count) }
}

#[no_mangle]
pub unsafe extern "C" fn memchr(memory: *const c_void, value: i32, count: usize) -> *mut c_void {
    let bytes = memory.cast::<u8>();
    for index in 0..count {
        if unsafe { read_volatile(bytes.add(index)) } == value as u8 {
            return unsafe { bytes.add(index).cast_mut().cast() };
        }
    }
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn strlen(text: *const i8) -> usize {
    let text = text.cast::<u8>();
    let mut length = 0;
    while unsafe { read_volatile(text.add(length)) } != 0 {
        length += 1;
    }
    length
}

#[no_mangle]
pub unsafe extern "C" fn strcmp(left: *const i8, right: *const i8) -> i32 {
    let left = left.cast::<u8>();
    let right = right.cast::<u8>();
    let mut index = 0;
    loop {
        let left_byte = unsafe { read_volatile(left.add(index)) };
        let right_byte = unsafe { read_volatile(right.add(index)) };
        if left_byte != right_byte || left_byte == 0 {
            return left_byte as i32 - right_byte as i32;
        }
        index += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strncmp(left: *const i8, right: *const i8, count: usize) -> i32 {
    let left = left.cast::<u8>();
    let right = right.cast::<u8>();
    for index in 0..count {
        let left_byte = unsafe { read_volatile(left.add(index)) };
        let right_byte = unsafe { read_volatile(right.add(index)) };
        if left_byte != right_byte || left_byte == 0 {
            return left_byte as i32 - right_byte as i32;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn strncpy(destination: *mut i8, source: *const i8, count: usize) -> *mut i8 {
    let destination_bytes = destination.cast::<u8>();
    let source_bytes = source.cast::<u8>();
    let mut index = 0;
    while index < count {
        let byte = unsafe { read_volatile(source_bytes.add(index)) };
        unsafe { write_volatile(destination_bytes.add(index), byte) };
        index += 1;
        if byte == 0 {
            break;
        }
    }
    while index < count {
        unsafe { write_volatile(destination_bytes.add(index), 0) };
        index += 1;
    }
    destination
}

#[no_mangle]
pub unsafe extern "C" fn strchr(text: *const i8, character: i32) -> *mut i8 {
    let bytes = text.cast::<u8>();
    let target = character as u8;
    let mut index = 0;
    loop {
        let byte = unsafe { read_volatile(bytes.add(index)) };
        if byte == target {
            return unsafe { bytes.add(index).cast_mut().cast() };
        }
        if byte == 0 {
            return core::ptr::null_mut();
        }
        index += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strstr(haystack: *const i8, needle: *const i8) -> *mut i8 {
    let haystack_bytes = haystack.cast::<u8>();
    let needle_bytes = needle.cast::<u8>();
    if unsafe { read_volatile(needle_bytes) } == 0 {
        return haystack.cast_mut();
    }
    let mut start_index = 0;
    loop {
        if unsafe { read_volatile(haystack_bytes.add(start_index)) } == 0 {
            return core::ptr::null_mut();
        }
        let mut needle_index = 0;
        loop {
            let haystack_byte = unsafe {
                read_volatile(haystack_bytes.add(start_index + needle_index))
            };
            let needle_byte = unsafe { read_volatile(needle_bytes.add(needle_index)) };
            if needle_byte == 0 {
                return unsafe { haystack_bytes.add(start_index).cast_mut().cast() };
            }
            if haystack_byte != needle_byte {
                break;
            }
            needle_index += 1;
        }
        start_index += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn __memcpy_chk(
    destination: *mut c_void,
    source: *const c_void,
    count: usize,
    destination_size: usize,
) -> *mut c_void {
    if count > destination_size {
        trap();
    }
    unsafe { memcpy(destination, source, count) }
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
