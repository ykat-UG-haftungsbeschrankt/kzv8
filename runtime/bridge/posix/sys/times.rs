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

use super::super::errno::{as_errno, EFAULT};

const SCHEDULER_TICK_NS: u64 = 10_000_000;

#[repr(C)]
struct Tms {
    user_time: i64,
    system_time: i64,
    child_user_time: i64,
    child_system_time: i64,
}

unsafe extern "C" {
    fn kz_host_time_ns() -> u64;
}

fn cpu_ticks(cpu_time_ns: u64) -> i64 {
    (cpu_time_ns / SCHEDULER_TICK_NS).min(i64::MAX as u64) as i64
}

#[no_mangle]
pub unsafe extern "C" fn times(buffer: *mut c_void) -> i64 {
    if buffer.is_null() {
        unsafe { crate::kernel::scheduler::thread::__errno_location().write(EFAULT) };
        return -1;
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => {
            unsafe {
                crate::kernel::scheduler::thread::__errno_location().write(as_errno(error));
            }
            return -1;
        }
    };
    let process_usage = ThreadManager::current_process_resource_usage_snapshot()
        .unwrap_or_else(|| process.resource_usage_snapshot());
    let child_usage = process.child_resource_usage_snapshot();
    let value = Tms {
        user_time: cpu_ticks(process_usage.user_cpu_time_ns),
        system_time: cpu_ticks(process_usage.system_cpu_time_ns),
        child_user_time: cpu_ticks(child_usage.user_cpu_time_ns),
        child_system_time: cpu_ticks(child_usage.system_cpu_time_ns),
    };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&value as *const Tms).cast::<u8>(),
            size_of::<Tms>(),
        )
    };
    if let Err(error) = process.address_space().write(buffer as usize, bytes) {
        unsafe {
            crate::kernel::scheduler::thread::__errno_location().write(as_errno(error));
        }
        return -1;
    }
    let elapsed = unsafe { kz_host_time_ns() } / SCHEDULER_TICK_NS;
    elapsed.min(i64::MAX as u64) as i64
}