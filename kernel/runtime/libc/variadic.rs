/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_int, c_void};

pub(crate) const VA_INT: c_int = 1;
pub(crate) const VA_UINT: c_int = 2;
pub(crate) const VA_LONG: c_int = 3;
pub(crate) const VA_ULONG: c_int = 4;
pub(crate) const VA_LLONG: c_int = 5;
pub(crate) const VA_ULLONG: c_int = 6;
pub(crate) const VA_POINTER: c_int = 7;
pub(crate) const VA_SIZE: c_int = 8;
pub(crate) const VA_PTRDIFF: c_int = 9;

unsafe extern "C" {
    fn kz_va_arg(arguments: *mut c_void, kind: c_int) -> usize;
    fn kz_va_copy(destination: *mut c_void, source: *mut c_void);
    fn kz_va_end(arguments: *mut c_void);
}

pub(crate) unsafe fn next(arguments: *mut c_void, kind: c_int) -> usize {
    unsafe { kz_va_arg(arguments, kind) }
}

pub(crate) unsafe fn copy(destination: *mut c_void, source: *mut c_void) {
    unsafe { kz_va_copy(destination, source) };
}

pub(crate) unsafe fn end(arguments: *mut c_void) {
    unsafe { kz_va_end(arguments) };
}