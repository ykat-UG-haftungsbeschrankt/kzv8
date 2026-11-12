/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_char, c_int};

use crate::kernel::runtime::runtime_libc::filesystem;

use super::super::errno::EINVAL;

#[no_mangle]
pub unsafe extern "C" fn ftok(path: *const c_char, project_id: c_int) -> i32 {
    if project_id == 0 {
        unsafe { crate::kernel::scheduler::thread::__errno_location().write(EINVAL) };
        return -1;
    }
    match filesystem::file_identity(path) {
        Ok((device, inode)) => {
            let key = ((project_id as u32 & 0xff) << 24)
                | ((inode as u32 & 0xffff) << 8)
                | (device as u32 & 0xff);
            key as i32
        }
        Err(error) => {
            unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
            -1
        }
    }
}