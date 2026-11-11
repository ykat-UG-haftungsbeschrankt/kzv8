/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_char, c_int};
use super::errno::{EINVAL, ENOSYS};

const O_WRONLY: c_int = 1;
const O_CREAT: c_int = 0o100;
const O_TRUNC: c_int = 0o1000;
const O_NONBLOCK: c_int = 0o4000;
const O_ACCMODE: c_int = 0o3;
const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;

unsafe extern "C" {
    fn kz_rust_open(path: *const c_char, flags: c_int, mode: u32) -> c_int;
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub unsafe extern "C" fn creat(path: *const c_char, mode: u32) -> c_int {
    unsafe { kz_rust_open(path, O_WRONLY | O_CREAT | O_TRUNC, mode) }
}

#[no_mangle]
pub unsafe extern "C" fn fcntl(
    file_descriptor: c_int,
    command: c_int,
    argument: usize,
) -> c_int {
    if crate::kernel::runtime::runtime_libc::network::is_socket_fd(file_descriptor)
        || crate::kernel::runtime::runtime_libc::network::is_packet_fd(file_descriptor)
    {
        return match command {
            F_GETFL => crate::kernel::runtime::runtime_libc::network::get_status_flags(
                file_descriptor,
            )
            .unwrap_or_else(|error| {
                set_errno(-(error as i32));
                -1
            }),
            F_SETFL => {
                if argument as c_int & !(O_NONBLOCK | O_ACCMODE) != 0 {
                    set_errno(EINVAL);
                    return -1;
                }
                match crate::kernel::runtime::runtime_libc::network::set_status_flags(
                    file_descriptor,
                    argument as c_int,
                ) {
                    Ok(()) => 0,
                    Err(error) => {
                        set_errno(-(error as i32));
                        -1
                    }
                }
            }
            _ => {
                set_errno(ENOSYS);
                -1
            }
        };
    }
    set_errno(ENOSYS);
    -1
}

// FIXME: The filesystem has no directory-descriptor-relative path resolution.
#[no_mangle]
pub unsafe extern "C" fn openat(
    _directory_descriptor: c_int,
    _path: *const c_char,
    _flags: c_int,
    _mode: u32,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub extern "C" fn posix_fadvise(
    _file_descriptor: c_int,
    _offset: i64,
    _length: i64,
    _advice: c_int,
) -> c_int {
    ENOSYS
}

#[no_mangle]
pub extern "C" fn posix_fallocate(
    _file_descriptor: c_int,
    _offset: i64,
    _length: i64,
) -> c_int {
    ENOSYS
}