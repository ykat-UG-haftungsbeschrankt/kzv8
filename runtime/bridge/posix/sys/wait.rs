/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_int, c_void};
use core::mem::size_of;

use crate::kernel::dl::AddressSpace;
use crate::kernel::arch::traits::CpuArchitecture as _;
use crate::kernel::error::Error;
use crate::kernel::process::{ChildExitStatus, Process};
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::signal::{CLD_CONTINUED, CLD_EXITED, CLD_KILLED, CLD_STOPPED, SIGCHLD};

use super::super::errno::{as_errno, EFAULT, EINVAL};

const WNOHANG: c_int = 0x0001;
const WUNTRACED: c_int = 0x0002;
const WEXITED: c_int = 0x0004;
const WCONTINUED: c_int = 0x0008;
const WNOWAIT: c_int = 0x0100_0000;
const WAITPID_OPTIONS: c_int = WNOHANG | WUNTRACED | WCONTINUED;
const WAITID_OPTIONS: c_int = WNOHANG | WEXITED | WUNTRACED | WCONTINUED | WNOWAIT;

#[repr(C)]
#[derive(Clone, Copy)]
union Sigval {
    integer: c_int,
    pointer: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CSignalInfo {
    signal_number: c_int,
    code: c_int,
    error: c_int,
    sender_pid: i32,
    sender_uid: u32,
    address: *mut c_void,
    status: c_int,
    value: Sigval,
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

fn validate_output<T>(process: &Process, output: *mut T) -> Result<(), Error> {
    if output.is_null() {
        return Ok(());
    }
    if process
        .address_space()
        .user_range_accessible(output as usize, size_of::<T>(), true)
    {
        Ok(())
    } else {
        Err(Error::Fault)
    }
}

fn write_user_value<T: Copy>(
    process: &Process,
    output: *mut T,
    value: &T,
) -> Result<(), Error> {
    if output.is_null() {
        return Ok(());
    }
    let bytes = unsafe {
        core::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>())
    };
    process.address_space().write(output as usize, bytes)
}

fn wait_for_child(
    process: &Process,
    process_id: i32,
    wait_exited: bool,
    wait_stopped: bool,
    wait_continued: bool,
    nohang: bool,
    nowait: bool,
) -> Result<Option<ChildExitStatus>, Error> {
    loop {
        match process.child_wait_status(
            process_id,
            wait_exited,
            wait_stopped,
            wait_continued,
            nowait,
        )? {
            Some(status) => return Ok(Some(status)),
            None if nohang => return Ok(None),
            None => {
                if !ThreadManager::switch_current_for_signal() {
                    crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
                }
                if crate::kernel::runtime::runtime_posix::signal::dispatch_pending_signals()
                    .is_some_and(|restart| !restart)
                {
                    return Err(Error::Intr);
                }
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn waitid(
    id_type: c_int,
    identifier: u32,
    information_pointer: *mut c_void,
    options: c_int,
) -> c_int {
    if information_pointer.is_null()
        || options & !WAITID_OPTIONS != 0
        || options & (WEXITED | WUNTRACED | WCONTINUED) == 0
    {
        set_errno(if information_pointer.is_null() {
            EFAULT
        } else {
            EINVAL
        });
        return -1;
    }
    let process_id = match id_type {
        0 => -1,
        1 if identifier != 0 && identifier <= i32::MAX as u32 => identifier as i32,
        2 if identifier == 0 => 0,
        2 if identifier <= i32::MAX as u32 => -(identifier as i32),
        _ => {
            set_errno(EINVAL);
            return -1;
        }
    };
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(as_errno(error));
            return -1;
        }
    };
    if let Err(error) = validate_output(&process, information_pointer.cast::<CSignalInfo>()) {
        set_errno(as_errno(error));
        return -1;
    }
    let status = match wait_for_child(
        &process,
        process_id,
        options & WEXITED != 0,
        options & WUNTRACED != 0,
        options & WCONTINUED != 0,
        options & WNOHANG != 0,
        options & WNOWAIT != 0,
    ) {
        Ok(status) => status,
        Err(error) => {
            set_errno(as_errno(error));
            return -1;
        }
    };
    let information = status.map_or(
        CSignalInfo {
            signal_number: 0,
            code: 0,
            error: 0,
            sender_pid: 0,
            sender_uid: 0,
            address: core::ptr::null_mut(),
            status: 0,
            value: Sigval { integer: 0 },
        },
        |status| CSignalInfo {
            signal_number: SIGCHLD as c_int,
            code: status.code,
            error: 0,
            sender_pid: status.process_id,
            sender_uid: 0,
            address: core::ptr::null_mut(),
            status: status.status,
            value: Sigval { integer: 0 },
        },
    );
    if let Err(error) = write_user_value(
        &process,
        information_pointer.cast::<CSignalInfo>(),
        &information,
    ) {
        set_errno(as_errno(error));
        return -1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn waitpid(
    process_id: i32,
    status: *mut c_int,
    options: c_int,
) -> i32 {
    if options & !WAITPID_OPTIONS != 0 {
        set_errno(EINVAL);
        return -1;
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(as_errno(error));
            return -1;
        }
    };
    if let Err(error) = validate_output(&process, status) {
        set_errno(as_errno(error));
        return -1;
    }
    let child = match wait_for_child(
        &process,
        process_id,
        true,
        options & WUNTRACED != 0,
        options & WCONTINUED != 0,
        options & WNOHANG != 0,
        false,
    ) {
        Ok(child) => child,
        Err(error) => {
            set_errno(as_errno(error));
            return -1;
        }
    };
    let Some(child) = child else {
        return 0;
    };
    if !status.is_null() {
        let encoded_status = wait_status(child);
        if let Err(error) = write_user_value(&process, status, &encoded_status) {
            set_errno(as_errno(error));
            return -1;
        }
    }
    child.process_id
}

fn wait_status(status: ChildExitStatus) -> c_int {
    match status.code {
        CLD_EXITED => (status.status & 0xff) << 8,
        CLD_KILLED => status.status & 0x7f,
        CLD_STOPPED => ((status.status & 0xff) << 8) | 0x7f,
        CLD_CONTINUED => 0xffff,
        _ => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn wait(status: *mut c_int) -> i32 {
    unsafe { waitpid(-1, status, 0) }
}