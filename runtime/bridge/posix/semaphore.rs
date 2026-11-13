/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_void, CStr};
use core::sync::atomic::{AtomicU32, Ordering};

use crate::kernel::scheduler::sync::IrqSpinLock;

use super::errno::{as_errno, EAGAIN, EFAULT, EINVAL, ENOMEM};

const O_CREAT: c_int = 0o100;
const O_EXCL: c_int = 0o200;
const SEM_VALUE_MAX: u32 = 0x7fff_ffff;
const CLOCK_REALTIME: c_int = 0;
const CLOCK_MONOTONIC: c_int = 1;
const CLOCK_MONOTONIC_RAW: c_int = 4;
const ETIMEDOUT: c_int = 110;

#[repr(C)]
#[derive(Clone, Copy)]
struct Timespec {
    seconds: i64,
    nanoseconds: i64,
}

struct NamedSemaphore {
    name: Vec<u8>,
    address: usize,
    references: usize,
    unlinked: bool,
}

static NAMED_SEMAPHORES: IrqSpinLock<Vec<NamedSemaphore>> = IrqSpinLock::new(Vec::new());

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn free(pointer: *mut c_void);
    fn sem_trywait(semaphore: *mut c_void) -> c_int;
    fn clock_gettime(clock_id: c_int, output: *mut Timespec) -> c_int;
    fn pthread_yield() -> c_int;
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

fn validate_name(name: &[u8]) -> Result<(), c_int> {
    if name.len() < 2 || name[0] != b'/' || name[1..].contains(&b'/') || name.contains(&0) {
        return Err(EINVAL);
    }
    Ok(())
}

fn copy_name(name: &[u8]) -> Result<Vec<u8>, c_int> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(name.len()).map_err(|_| ENOMEM)?;
    copy.extend_from_slice(name);
    Ok(copy)
}

fn named_semaphore_open(
    name: &[u8],
    flags: c_int,
    value: u32,
) -> Result<*mut c_void, c_int> {
    validate_name(name)?;
    if flags & !(O_CREAT | O_EXCL) != 0 || value > SEM_VALUE_MAX {
        return Err(EINVAL);
    }
    let mut semaphores = NAMED_SEMAPHORES.lock();
    if let Some(semaphore) = semaphores
        .iter_mut()
        .find(|semaphore| !semaphore.unlinked && semaphore.name == name)
    {
        if flags & (O_CREAT | O_EXCL) == (O_CREAT | O_EXCL) {
            return Err(as_errno(crate::kernel::error::Error::Exist));
        }
        semaphore.references = semaphore.references.checked_add(1).ok_or(ENOMEM)?;
        return Ok(semaphore.address as *mut c_void);
    }
    if flags & O_CREAT == 0 {
        return Err(as_errno(crate::kernel::error::Error::NoEnt));
    }
    let stored_name = copy_name(name)?;
    let semaphore = unsafe { malloc(core::mem::size_of::<AtomicU32>()) };
    if semaphore.is_null() {
        return Err(ENOMEM);
    }
    unsafe {
        semaphore
            .cast::<AtomicU32>()
            .write(AtomicU32::new(value));
    }
    if semaphores.try_reserve(1).is_err() {
        unsafe { free(semaphore) };
        return Err(ENOMEM);
    }
    semaphores.push(NamedSemaphore {
        name: stored_name,
        address: semaphore as usize,
        references: 1,
        unlinked: false,
    });
    Ok(semaphore)
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_sem_open(
    name: *const c_char,
    flags: c_int,
    _mode: u32,
    value: u32,
) -> *mut c_void {
    if name.is_null() {
        set_errno(EFAULT);
        return core::ptr::null_mut();
    }
    match named_semaphore_open(unsafe { CStr::from_ptr(name) }.to_bytes(), flags, value) {
        Ok(semaphore) => semaphore,
        Err(error) => {
            set_errno(error);
            core::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn sem_getvalue(semaphore: *mut u32, output: *mut c_int) -> c_int {
    if semaphore.is_null() || output.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let value = unsafe { &*semaphore.cast::<AtomicU32>() }.load(Ordering::Acquire);
    unsafe { output.write(value as c_int) };
    0
}

// FIXME: The kernel has no named-semaphore store or timed semaphore wait queue.
#[no_mangle]
pub unsafe extern "C" fn sem_close(semaphore: *mut c_void) -> c_int {
    if semaphore.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let mut semaphores = NAMED_SEMAPHORES.lock();
    let Some(index) = semaphores
        .iter()
        .position(|entry| entry.address == semaphore as usize)
    else {
        set_errno(EINVAL);
        return -1;
    };
    if semaphores[index].references == 0 {
        set_errno(EINVAL);
        return -1;
    }
    semaphores[index].references = semaphores[index].references.saturating_sub(1);
    if semaphores[index].references == 0 && semaphores[index].unlinked {
        let semaphore = semaphores.remove(index);
        drop(semaphores);
        unsafe { free(semaphore.address as *mut c_void) };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn sem_timedwait(
    semaphore: *mut c_void,
    timeout: *const c_void,
) -> c_int {
    unsafe { timed_wait(semaphore, CLOCK_REALTIME, timeout) }
}

#[no_mangle]
pub unsafe extern "C" fn sem_clockwait(
    semaphore: *mut c_void,
    clock_id: c_int,
    timeout: *const c_void,
) -> c_int {
    unsafe { timed_wait(semaphore, clock_id, timeout) }
}

#[no_mangle]
pub unsafe extern "C" fn sem_unlink(name: *const c_char) -> c_int {
    if name.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let name = unsafe { CStr::from_ptr(name) }.to_bytes();
    if let Err(error) = validate_name(name) {
        set_errno(error);
        return -1;
    }
    let mut semaphores = NAMED_SEMAPHORES.lock();
    let Some(index) = semaphores
        .iter()
        .position(|entry| !entry.unlinked && entry.name == name)
    else {
        set_errno(as_errno(crate::kernel::error::Error::NoEnt));
        return -1;
    };
    semaphores[index].unlinked = true;
    if semaphores[index].references == 0 {
        let semaphore = semaphores.remove(index);
        drop(semaphores);
        unsafe { free(semaphore.address as *mut c_void) };
    }
    0
}

unsafe fn timed_wait(
    semaphore: *mut c_void,
    clock_id: c_int,
    timeout: *const c_void,
) -> c_int {
    if semaphore.is_null() || timeout.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    if !matches!(clock_id, CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW) {
        set_errno(EINVAL);
        return -1;
    }
    let timeout = unsafe { timeout.cast::<Timespec>().read_unaligned() };
    if timeout.seconds < 0 || !(0..1_000_000_000).contains(&timeout.nanoseconds) {
        set_errno(EINVAL);
        return -1;
    }
    let mut now = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    if unsafe { clock_gettime(clock_id, &mut now) } != 0 {
        return -1;
    }
    let deadline_ns = i128::from(timeout.seconds) * 1_000_000_000
        + i128::from(timeout.nanoseconds);
    loop {
        crate::kernel::scheduler::thread::ThreadManager::cancel_current_if_requested();
        if unsafe { sem_trywait(semaphore) } == 0 {
            return 0;
        }
        let error = unsafe { crate::kernel::scheduler::thread::__errno_location().read() };
        if error != EAGAIN {
            return -1;
        }
        if unsafe { clock_gettime(clock_id, &mut now) } != 0 {
            return -1;
        }
        let now_ns = i128::from(now.seconds) * 1_000_000_000 + i128::from(now.nanoseconds);
        if now_ns >= deadline_ns {
            set_errno(ETIMEDOUT);
            return -1;
        }
        unsafe { pthread_yield() };
    }
}