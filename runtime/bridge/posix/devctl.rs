/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_int, c_void};
use core::mem::size_of;

use crate::kernel::dl::AddressSpace;
use crate::kernel::scheduler::thread::ThreadManager;

use super::errno::{as_errno, EFAULT};

unsafe extern "C" {
    fn kz_rust_ioctl(file_descriptor: c_int, request: usize, argument: *mut c_void) -> c_int;
}

// FIXME: The kernel has no device-control dispatch interface for open descriptors.
#[no_mangle]
pub unsafe extern "C" fn posix_devctl(
    file_descriptor: c_int,
    command: c_int,
    control_data: *mut c_void,
    control_size: usize,
    device_information: *mut c_int,
) -> c_int {
    if control_data.is_null() && control_size != 0 {
        return EFAULT;
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => return as_errno(error),
    };
    if !control_data.is_null()
        && !process
            .address_space()
            .user_range_accessible(control_data as usize, control_size, true)
    {
        return EFAULT;
    }
    if !device_information.is_null()
        && !process
            .address_space()
            .user_range_accessible(device_information as usize, size_of::<c_int>(), true)
    {
        return EFAULT;
    }
    if unsafe { kz_rust_ioctl(file_descriptor, command as u32 as usize, control_data) } != 0 {
        return unsafe { crate::kernel::scheduler::thread::__errno_location().read() };
    }
    if !device_information.is_null() {
        let value = 0;
        let bytes = unsafe {
            core::slice::from_raw_parts((&value as *const c_int).cast::<u8>(), size_of::<c_int>())
        };
        if let Err(error) = process.address_space().write(device_information as usize, bytes) {
            return as_errno(error);
        }
    }
    0
}