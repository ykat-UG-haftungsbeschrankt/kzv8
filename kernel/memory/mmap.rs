/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::c_void;

const PAGE_SIZE: usize = 4096;

const EINVAL: i32 = 22;
const ENOMEM: i32 = 12;
const MADV_WILLNEED: i32 = 3;
const MADV_DONTNEED: i32 = 4;
const MADV_FREE: i32 = 8;

const PROT_READ: i32 = 1;
const PROT_WRITE: i32 = 2;
const PROT_EXEC: i32 = 4;
const PROT_READ_WRITE: i32 = PROT_READ | PROT_WRITE;
const PROT_READ_EXEC: i32 = PROT_READ | PROT_EXEC;
const PROT_WRITE_EXEC: i32 = PROT_WRITE | PROT_EXEC;
const PROT_READ_WRITE_EXEC: i32 = PROT_READ | PROT_WRITE | PROT_EXEC;

const MAP_SHARED: i32 = 1;
const MAP_PRIVATE: i32 = 2;
const MAP_ANONYMOUS: i32 = 0x20;
const MAP_NORESERVE: i32 = 0x4000;
const MAP_FIXED: i32 = 0x10;

fn set_errno(error: i32) {
    unsafe { crate::kernel_thread::__errno_location().write(error) };
}

fn fail(error: i32) -> *mut c_void {
    set_errno(error);
    usize::MAX as *mut c_void
}

fn rounded_length(length: usize) -> Option<usize> {
    length
        .checked_add(PAGE_SIZE - 1)
        .map(|value| value & !(PAGE_SIZE - 1))
}

fn page_permissions(protection: i32) -> Option<i32> {
    match protection {
        0 => Some(0),
        PROT_READ => Some(1),
        PROT_WRITE | PROT_READ_WRITE => Some(2),
        PROT_EXEC | PROT_READ_EXEC => Some(4),
        PROT_WRITE_EXEC | PROT_READ_WRITE_EXEC => Some(3),
        _ => None,
    }
}

#[no_mangle]
pub unsafe extern "C" fn mmap(
    address_hint: *mut c_void,
    length: usize,
    protection: i32,
    flags: i32,
    file_descriptor: i32,
    offset: i64,
) -> *mut c_void {
    let sharing = flags & (MAP_PRIVATE | MAP_SHARED);
    let supported_flags = MAP_PRIVATE | MAP_SHARED | MAP_ANONYMOUS | MAP_NORESERVE | MAP_FIXED;
    if length == 0
        || protection & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0
        || flags & !supported_flags != 0
        || flags & MAP_ANONYMOUS == 0
        || !matches!(sharing, MAP_PRIVATE | MAP_SHARED)
        || file_descriptor != -1
        || offset != 0
    {
        return fail(EINVAL);
    }
    let Some(permissions) = page_permissions(protection) else {
        return fail(EINVAL);
    };
    let Some(length) = rounded_length(length) else {
        return fail(ENOMEM);
    };
    if flags & MAP_FIXED != 0 {
        if address_hint.is_null() || permissions != 0 {
            return fail(EINVAL);
        }
        if !crate::memory::decommit(address_hint as usize, length) {
            return fail(ENOMEM);
        }
        return address_hint;
    }
    let Some(address) = crate::memory::reserve(length, PAGE_SIZE, permissions) else {
        return fail(ENOMEM);
    };
    address as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn mmap64(
    address_hint: *mut c_void,
    length: usize,
    protection: i32,
    flags: i32,
    file_descriptor: i32,
    offset: i64,
) -> *mut c_void {
    unsafe { mmap(address_hint, length, protection, flags, file_descriptor, offset) }
}

#[no_mangle]
pub unsafe extern "C" fn mprotect(
    address: *mut c_void,
    length: usize,
    protection: i32,
) -> i32 {
    if address.is_null() || address as usize % PAGE_SIZE != 0 || length == 0 {
        set_errno(EINVAL);
        return -1;
    }
    let Some(permissions) = page_permissions(protection) else {
        set_errno(EINVAL);
        return -1;
    };
    let address = address as usize;
    if crate::memory::set_permissions(address, length, permissions) {
        return 0;
    }
    if crate::arch::protect_kernel_image_pages(address, length, permissions) {
        return 0;
    }
    set_errno(ENOMEM);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn madvise(
    address: *mut c_void,
    length: usize,
    advice: i32,
) -> i32 {
    if address.is_null()
        || address as usize % PAGE_SIZE != 0
        || length == 0
        || rounded_length(length).is_none()
        || !matches!(advice, MADV_WILLNEED | MADV_DONTNEED | MADV_FREE)
    {
        set_errno(EINVAL);
        return -1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn munmap(address: *mut c_void, length: usize) -> i32 {
    let address = address as usize;
    if address == 0 || address % PAGE_SIZE != 0 || length == 0 {
        set_errno(EINVAL);
        return -1;
    }
    let Some(length) = rounded_length(length) else {
        set_errno(EINVAL);
        return -1;
    };
    if !crate::memory::release_range(address, length) {
        set_errno(EINVAL);
        return -1;
    }
    0
}
