/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_int, c_void};
use core::mem::{size_of, MaybeUninit};

use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::scheduler::thread::ThreadManager;

use super::super::errno::{as_errno, ENOSYS};

#[repr(C)]
#[derive(Clone, Copy)]
struct RLimit {
    current: u64,
    maximum: u64,
}

const PRIO_PROCESS: c_int = 0;

fn process_for_priority(which: c_int, who: u32) -> Result<alloc::sync::Arc<Process>, Error> {
    if which != PRIO_PROCESS {
        return Err(Error::NoSys);
    }
    let process = ThreadManager::current_process()?;
    if who == 0 || i32::try_from(who).ok() == Some(process.pid()) {
        Ok(process)
    } else {
        Err(Error::Srch)
    }
}

fn set_errno(error: c_int) {
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
}

// FIXME: The kernel runtime has no process resource accounting or limit facility.
#[no_mangle]
pub unsafe extern "C" fn getpriority(which: c_int, who: u32) -> c_int {
    match process_for_priority(which, who) {
        Ok(process) => process.nice_priority(),
        Err(error) => {
            set_errno(as_errno(error));
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn getrlimit(_resource: c_int, _limit: *mut core::ffi::c_void) -> c_int {
    if _limit.is_null() {
        return fail(Error::Fault);
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => return fail(error),
    };
    let (current, maximum) = match process.resource_limit(_resource) {
        Ok(limit) => limit,
        Err(error) => return fail(error),
    };
    let value = RLimit { current, maximum };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&value as *const RLimit).cast::<u8>(),
            size_of::<RLimit>(),
        )
    };
    match process.address_space().write(_limit as usize, bytes) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn getrusage(_who: c_int, _usage: *mut core::ffi::c_void) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn setrlimit(_resource: c_int, _limit: *const c_void) -> c_int {
    if _limit.is_null() {
        return fail(Error::Fault);
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => return fail(error),
    };
    let mut value = MaybeUninit::<RLimit>::uninit();
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), size_of::<RLimit>())
    };
    if let Err(error) = process.address_space().read(_limit as usize, bytes) {
        return fail(error);
    }
    let value = unsafe { value.assume_init() };
    match process.set_resource_limit(_resource, value.current, value.maximum) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn setpriority(which: c_int, who: u32, priority: c_int) -> c_int {
    let process = match process_for_priority(which, who) {
        Ok(process) => process,
        Err(error) => {
            set_errno(as_errno(error));
            return -1;
        }
    };
    match process.set_nice_priority(priority) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(as_errno(error));
            -1
        }
    }
}

fn fail(error: Error) -> c_int {
    set_errno(as_errno(error));
    -1
}