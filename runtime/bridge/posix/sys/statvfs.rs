/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_char, c_int, c_void};
use core::mem::size_of;

use crate::kernel::dl::AddressSpace;
use crate::kernel::runtime::runtime_libc::filesystem::{self, FileSystemCapacity};
use crate::kernel::scheduler::thread::ThreadManager;

use super::super::errno::{as_errno, EFAULT};

#[repr(C)]
#[derive(Clone, Copy)]
struct CStatVfs {
    block_size: usize,
    fragment_size: usize,
    blocks: u64,
    free_blocks: u64,
    available_blocks: u64,
    files: u64,
    free_files: u64,
    available_files: u64,
    filesystem_id: usize,
    flags: usize,
    name_max: usize,
}

fn set_errno(error: c_int) {
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
}

fn statvfs_value(capacity: FileSystemCapacity) -> CStatVfs {
    CStatVfs {
        block_size: capacity.block_size as usize,
        fragment_size: capacity.fragment_size as usize,
        blocks: capacity.blocks,
        free_blocks: capacity.free_blocks,
        available_blocks: capacity.available_blocks,
        files: capacity.files,
        free_files: capacity.free_files,
        available_files: capacity.available_files,
        filesystem_id: capacity.filesystem_id as usize,
        flags: capacity.flags as usize,
        name_max: capacity.name_max as usize,
    }
}

fn write_status(output: *mut c_void, status: CStatVfs) -> c_int {
    if output.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(as_errno(error));
            return -1;
        }
    };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&status as *const CStatVfs).cast::<u8>(),
            size_of::<CStatVfs>(),
        )
    };
    match process.address_space().write(output as usize, bytes) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(as_errno(error));
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn fstatvfs(file_descriptor: c_int, status: *mut c_void) -> c_int {
    match filesystem::capacity_for_descriptor(file_descriptor) {
        Ok(capacity) => write_status(status, statvfs_value(capacity)),
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn statvfs(path: *const c_char, status: *mut c_void) -> c_int {
    match filesystem::capacity_for_path(path) {
        Ok(capacity) => write_status(status, statvfs_value(capacity)),
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}