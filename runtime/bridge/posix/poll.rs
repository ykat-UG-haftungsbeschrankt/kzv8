/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_int, c_void};
use core::ptr;

use crate::kernel::error::Error;
use crate::kernel::runtime::runtime_libc::filesystem;

const MAX_POLL_DESCRIPTORS: usize = 1024;
const POLLNVAL: i16 = 0x020;

#[repr(C)]
#[derive(Clone, Copy)]
struct PollFd {
    file_descriptor: c_int,
    events: i16,
    revents: i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Timespec {
    seconds: i64,
    nanoseconds: i64,
}

fn set_errno(error: Error) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(-(error as i32));
    }
}

fn poll_inner(
    descriptors: *mut PollFd,
    descriptor_count: usize,
    timeout_ns: Option<u64>,
) -> c_int {
    if descriptor_count > MAX_POLL_DESCRIPTORS || (descriptor_count != 0 && descriptors.is_null()) {
        set_errno(if descriptors.is_null() && descriptor_count != 0 {
            Error::Fault
        } else {
            Error::Inval
        });
        return -1;
    }
    let started = now_ns();
    loop {
        let mut ready_count = 0;
        let descriptors = if descriptor_count == 0 {
            &mut [][..]
        } else {
            unsafe { core::slice::from_raw_parts_mut(descriptors, descriptor_count) }
        };
        for descriptor in descriptors {
            descriptor.revents = 0;
            if descriptor.file_descriptor < 0 {
                continue;
            }
            descriptor.revents = filesystem::poll_readiness(
                descriptor.file_descriptor,
                descriptor.events,
            );
            if descriptor.revents != 0 {
                ready_count += 1;
            }
        }
        if ready_count != 0 {
            return ready_count;
        }
        if let Some(timeout_ns) = timeout_ns {
            if now_ns().saturating_sub(started) >= timeout_ns {
                return 0;
            }
        }
        crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
    }
}

#[no_mangle]
pub unsafe extern "C" fn poll(
    descriptors: *mut c_void,
    descriptor_count: u64,
    timeout: c_int,
) -> c_int {
    if timeout < -1 {
        set_errno(Error::Inval);
        return -1;
    }
    let timeout_ns = (timeout >= 0)
        .then_some((timeout as u64).saturating_mul(1_000_000));
    poll_inner(
        descriptors.cast::<PollFd>(),
        match usize::try_from(descriptor_count) {
            Ok(count) => count,
            Err(_) => {
                set_errno(Error::Inval);
                return -1;
            }
        },
        timeout_ns,
    )
}

#[no_mangle]
pub unsafe extern "C" fn ppoll(
    descriptors: *mut c_void,
    descriptor_count: u64,
    timeout: *const c_void,
    _signal_mask: *const c_void,
) -> c_int {
    let timeout_ns = if timeout.is_null() {
        None
    } else {
        let timespec = unsafe { ptr::read_unaligned(timeout.cast::<Timespec>()) };
        if timespec.seconds < 0
            || !(0..1_000_000_000).contains(&timespec.nanoseconds)
        {
            set_errno(Error::Inval);
            return -1;
        }
        Some(
            (timespec.seconds as u64)
                .saturating_mul(1_000_000_000)
                .saturating_add(timespec.nanoseconds as u64),
        )
    };
    poll_inner(
        descriptors.cast::<PollFd>(),
        match usize::try_from(descriptor_count) {
            Ok(count) => count,
            Err(_) => {
                set_errno(Error::Inval);
                return -1;
            }
        },
        timeout_ns,
    )
}

fn now_ns() -> u64 {
    crate::kernel::arch::TimerArchitecture::now_ns()
        .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns)
}