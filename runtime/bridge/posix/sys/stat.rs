/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_char, c_int, c_void};
use super::super::errno::EINVAL;

unsafe extern "C" {
    fn stat(path: *const c_char, status: *mut c_void) -> c_int;
    fn kz_rust_statat(
        directory_descriptor: c_int,
        path: *const c_char,
        status: *mut c_void,
        flags: c_int,
    ) -> c_int;
    fn kz_rust_lstat(path: *const c_char, status: *mut c_void) -> c_int;
    fn kz_rust_mkdirat(
        directory_descriptor: c_int,
        path: *const c_char,
        mode: u32,
    ) -> c_int;
    fn kz_rust_mkfifo(path: *const c_char, mode: u32) -> c_int;
    fn kz_rust_mkfifoat(
        directory_descriptor: c_int,
        path: *const c_char,
        mode: u32,
    ) -> c_int;
    fn kz_rust_mknod(path: *const c_char, mode: u32, device: u64) -> c_int;
    fn kz_rust_mknodat(
        directory_descriptor: c_int,
        path: *const c_char,
        mode: u32,
        device: u64,
    ) -> c_int;
    fn kz_rust_chmod(path: *const c_char, mode: u32) -> c_int;
    fn kz_rust_fchmod(file_descriptor: c_int, mode: u32) -> c_int;
    fn kz_rust_chmodat(
        directory_descriptor: c_int,
        path: *const c_char,
        mode: u32,
        flags: c_int,
    ) -> c_int;
    fn kz_rust_futimens(file_descriptor: c_int, times: *const c_void) -> c_int;
    fn kz_rust_utimensat(
        directory_descriptor: c_int,
        path: *const c_char,
        times: *const c_void,
        flags: c_int,
    ) -> c_int;
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub unsafe extern "C" fn chmod(path: *const c_char, mode: u32) -> c_int {
    unsafe { kz_rust_chmod(path, mode) }
}

#[no_mangle]
pub extern "C" fn fchmod(file_descriptor: c_int, mode: u32) -> c_int {
    unsafe { kz_rust_fchmod(file_descriptor, mode) }
}

#[no_mangle]
pub unsafe extern "C" fn fchmodat(
    directory_descriptor: c_int,
    path: *const c_char,
    mode: u32,
    flags: c_int,
) -> c_int {
    if flags & !0x100 != 0 {
        set_errno(EINVAL);
        return -1;
    }
    unsafe { kz_rust_chmodat(directory_descriptor, path, mode, flags) }
}

#[no_mangle]
pub unsafe extern "C" fn fstatat(
    directory_descriptor: c_int,
    path: *const c_char,
    status: *mut c_void,
    flags: c_int,
) -> c_int {
    if flags & !0x100 != 0 {
        set_errno(super::super::errno::EINVAL);
        return -1;
    }
    unsafe { kz_rust_statat(directory_descriptor, path, status, flags) }
}

#[no_mangle]
pub unsafe extern "C" fn futimens(file_descriptor: c_int, times: *const c_void) -> c_int {
    unsafe { kz_rust_futimens(file_descriptor, times) }
}

#[no_mangle]
pub unsafe extern "C" fn lstat(path: *const c_char, status: *mut c_void) -> c_int {
    unsafe { kz_rust_lstat(path, status) }
}

#[no_mangle]
pub unsafe extern "C" fn mkdirat(
    directory_descriptor: c_int,
    path: *const c_char,
    mode: u32,
) -> c_int {
    unsafe { kz_rust_mkdirat(directory_descriptor, path, mode) }
}

#[no_mangle]
pub unsafe extern "C" fn mkfifo(path: *const c_char, mode: u32) -> c_int {
    unsafe { kz_rust_mkfifo(path, mode) }
}

#[no_mangle]
pub unsafe extern "C" fn mkfifoat(
    directory_descriptor: c_int,
    path: *const c_char,
    mode: u32,
) -> c_int {
    unsafe { kz_rust_mkfifoat(directory_descriptor, path, mode) }
}

#[no_mangle]
pub unsafe extern "C" fn mknod(path: *const c_char, mode: u32, device: u64) -> c_int {
    unsafe { kz_rust_mknod(path, mode, device) }
}

#[no_mangle]
pub unsafe extern "C" fn mknodat(
    directory_descriptor: c_int,
    path: *const c_char,
    mode: u32,
    device: u64,
) -> c_int {
    unsafe { kz_rust_mknodat(directory_descriptor, path, mode, device) }
}

#[no_mangle]
pub extern "C" fn umask(_mask: u32) -> u32 {
    match crate::kernel::scheduler::thread::ThreadManager::current_process() {
        Ok(process) => process.set_umask(_mask),
        Err(error) => {
            set_errno(-(error as i32));
            u32::MAX
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn utimensat(
    directory_descriptor: c_int,
    path: *const c_char,
    times: *const c_void,
    flags: c_int,
) -> c_int {
    unsafe { kz_rust_utimensat(directory_descriptor, path, times, flags) }
}