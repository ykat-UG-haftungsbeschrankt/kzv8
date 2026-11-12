/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::ffi::{c_int, c_void};
use core::mem::{size_of, MaybeUninit};

use crate::kernel::arch::traits::TimerArchitecture as _;
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::scheduler::thread::ThreadManager;

use super::super::errno::as_errno;

const FD_SETSIZE: usize = 1024;
const FD_WORD_BITS: usize = usize::BITS as usize;
const FD_WORD_COUNT: usize = FD_SETSIZE / FD_WORD_BITS;
const POLLIN: i16 = 0x001;
const POLLPRI: i16 = 0x002;
const POLLOUT: i16 = 0x004;
const POLLERR: i16 = 0x008;
const POLLHUP: i16 = 0x010;
const POLLNVAL: i16 = 0x020;
const SIGNAL_SET_WORDS: usize = 16;

#[repr(C)]
#[derive(Clone, Copy)]
struct FdSet {
    words: [usize; FD_WORD_COUNT],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PollFd {
    file_descriptor: c_int,
    events: i16,
    revents: i16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Timeval {
    seconds: i64,
    microseconds: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Timespec {
    seconds: i64,
    nanoseconds: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CSignalSet {
    words: [u64; SIGNAL_SET_WORDS],
}

unsafe extern "C" {
    fn ppoll(
        descriptors: *mut c_void,
        descriptor_count: u64,
        timeout: *const c_void,
        signal_mask: *const c_void,
    ) -> c_int;
}

fn set_errno(error: Error) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(as_errno(error));
    }
}

fn monotonic_now_ns() -> u64 {
    crate::kernel::arch::TimerArchitecture::now_ns()
        .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns)
}

fn read_user_value<T: Copy>(process: &Process, pointer: *const T) -> Result<T, Error> {
    if pointer.is_null() {
        return Err(Error::Fault);
    }
    let mut value = MaybeUninit::<T>::uninit();
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), size_of::<T>())
    };
    process.address_space().read(pointer as usize, bytes)?;
    Ok(unsafe { value.assume_init() })
}

fn validate_output<T>(process: &Process, pointer: *mut T) -> Result<(), Error> {
    if pointer.is_null() {
        return Ok(());
    }
    if process
        .address_space()
        .user_range_accessible(pointer as usize, size_of::<T>(), true)
    {
        Ok(())
    } else {
        Err(Error::Fault)
    }
}

fn write_user_value<T: Copy>(
    process: &Process,
    pointer: *mut T,
    value: &T,
) -> Result<(), Error> {
    if pointer.is_null() {
        return Ok(());
    }
    let bytes = unsafe {
        core::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>())
    };
    process.address_space().write(pointer as usize, bytes)
}

fn fd_is_set(set: Option<FdSet>, descriptor: usize) -> bool {
    set.is_some_and(|set| {
        set.words[descriptor / FD_WORD_BITS] & (1usize << (descriptor % FD_WORD_BITS)) != 0
    })
}

fn select_inner(
    process: &Process,
    descriptor_count: c_int,
    read_pointer: *mut c_void,
    write_pointer: *mut c_void,
    error_pointer: *mut c_void,
    read_timeout: Option<(*mut c_void, u64)>,
    timespec: Option<Timespec>,
    signal_mask: *const CSignalSet,
) -> c_int {
    if descriptor_count < 0 || descriptor_count as usize > FD_SETSIZE {
        set_errno(Error::Inval);
        return -1;
    }
    let read_set = if read_pointer.is_null() {
        None
    } else {
        match read_user_value::<FdSet>(process, read_pointer.cast()) {
            Ok(set) => Some(set),
            Err(error) => {
                set_errno(error);
                return -1;
            }
        }
    };
    let write_set = if write_pointer.is_null() {
        None
    } else {
        match read_user_value::<FdSet>(process, write_pointer.cast()) {
            Ok(set) => Some(set),
            Err(error) => {
                set_errno(error);
                return -1;
            }
        }
    };
    let error_set = if error_pointer.is_null() {
        None
    } else {
        match read_user_value::<FdSet>(process, error_pointer.cast()) {
            Ok(set) => Some(set),
            Err(error) => {
                set_errno(error);
                return -1;
            }
        }
    };
    for result in [
        validate_output(process, read_pointer.cast::<FdSet>()),
        validate_output(process, write_pointer.cast::<FdSet>()),
        validate_output(process, error_pointer.cast::<FdSet>()),
    ] {
        if let Err(error) = result {
            set_errno(error);
            return -1;
        }
    }

    let mut poll_descriptors = Vec::new();
    if poll_descriptors
        .try_reserve_exact(descriptor_count as usize)
        .is_err()
    {
        set_errno(Error::NoMem);
        return -1;
    }
    for descriptor in 0..descriptor_count as usize {
        let mut events = 0;
        if fd_is_set(read_set, descriptor) {
            events |= POLLIN;
        }
        if fd_is_set(write_set, descriptor) {
            events |= POLLOUT;
        }
        if fd_is_set(error_set, descriptor) {
            events |= POLLPRI;
        }
        if events != 0 {
            poll_descriptors.push(PollFd {
                file_descriptor: descriptor as c_int,
                events,
                revents: 0,
            });
        }
    }

    let timeout = timespec.map(|value| &value as *const Timespec as *const c_void)
        .unwrap_or(core::ptr::null());
    let wait_started_ns = monotonic_now_ns();
    let result = unsafe {
        ppoll(
            poll_descriptors.as_mut_ptr().cast(),
            poll_descriptors.len() as u64,
            timeout,
            signal_mask.cast(),
        )
    };
    if result < 0 {
        return -1;
    }

    let mut updated_read = FdSet {
        words: [0; FD_WORD_COUNT],
    };
    let mut updated_write = updated_read;
    let mut updated_error = updated_read;
    let mut ready_descriptors = 0;
    for descriptor in &poll_descriptors {
        if descriptor.revents & POLLNVAL != 0 {
            set_errno(Error::Badf);
            return -1;
        }
        let readable = descriptor.revents & (POLLIN | POLLHUP | POLLERR) != 0;
        let writable = descriptor.revents & (POLLOUT | POLLERR) != 0;
        let exceptional = descriptor.revents & POLLPRI != 0;
        let fd = descriptor.file_descriptor as usize;
        if readable && fd_is_set(read_set, fd) {
            updated_read.words[fd / FD_WORD_BITS] |= 1usize << (fd % FD_WORD_BITS);
            ready_descriptors += 1;
        }
        if writable && fd_is_set(write_set, fd) {
            updated_write.words[fd / FD_WORD_BITS] |= 1usize << (fd % FD_WORD_BITS);
            ready_descriptors += 1;
        }
        if exceptional && fd_is_set(error_set, fd) {
            updated_error.words[fd / FD_WORD_BITS] |= 1usize << (fd % FD_WORD_BITS);
            ready_descriptors += 1;
        }
    }
    for (pointer, value) in [
        (read_pointer.cast::<FdSet>(), &updated_read),
        (write_pointer.cast::<FdSet>(), &updated_write),
        (error_pointer.cast::<FdSet>(), &updated_error),
    ] {
        if let Err(error) = write_user_value(process, pointer, value) {
            set_errno(error);
            return -1;
        }
    }
    if let Some((pointer, requested_ns)) = read_timeout {
        let elapsed_ns = monotonic_now_ns().saturating_sub(wait_started_ns);
        let remaining_ns = requested_ns.saturating_sub(elapsed_ns);
        let updated = Timeval {
            seconds: (remaining_ns / 1_000_000_000) as i64,
            microseconds: ((remaining_ns % 1_000_000_000) / 1_000) as i64,
        };
        if let Err(error) = write_user_value(process, pointer.cast(), &updated) {
            set_errno(error);
            return -1;
        }
    }
    ready_descriptors
}

#[no_mangle]
pub unsafe extern "C" fn select(
    descriptor_count: c_int,
    read_set: *mut c_void,
    write_set: *mut c_void,
    error_set: *mut c_void,
    timeout: *mut c_void,
) -> c_int {
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let timeval = if timeout.is_null() {
        None
    } else {
        match read_user_value::<Timeval>(&process, timeout.cast()) {
            Ok(value) if value.seconds >= 0 && (0..1_000_000).contains(&value.microseconds) => {
                let timeout_ns = (value.seconds as u64)
                    .saturating_mul(1_000_000_000)
                    .saturating_add((value.microseconds as u64).saturating_mul(1_000));
                Some((value, timeout_ns))
            }
            Ok(_) => {
                set_errno(Error::Inval);
                return -1;
            }
            Err(error) => {
                set_errno(error);
                return -1;
            }
        }
    };
    if let Some((_, _)) = timeval {
        if let Err(error) = validate_output(&process, timeout.cast::<Timeval>()) {
            set_errno(error);
            return -1;
        }
    }
    let timespec = timeval.map(|(_, timeout_ns)| Timespec {
        seconds: (timeout_ns / 1_000_000_000) as i64,
        nanoseconds: (timeout_ns % 1_000_000_000) as i64,
    });
    let result = select_inner(
        &process,
        descriptor_count,
        read_set,
        write_set,
        error_set,
        timeval.map(|(_, timeout_ns)| (timeout, timeout_ns)),
        timespec,
        core::ptr::null(),
    );
    result
}

#[no_mangle]
pub unsafe extern "C" fn pselect(
    descriptor_count: c_int,
    read_set: *mut c_void,
    write_set: *mut c_void,
    error_set: *mut c_void,
    timeout: *const c_void,
    signal_mask: *const c_void,
) -> c_int {
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let timespec = if timeout.is_null() {
        None
    } else {
        match read_user_value::<Timespec>(&process, timeout.cast()) {
            Ok(value) if value.seconds >= 0 && (0..1_000_000_000).contains(&value.nanoseconds) => {
                Some(value)
            }
            Ok(_) => {
                set_errno(Error::Inval);
                return -1;
            }
            Err(error) => {
                set_errno(error);
                return -1;
            }
        }
    };
    let signal_mask = if signal_mask.is_null() {
        None
    } else {
        match read_user_value::<CSignalSet>(&process, signal_mask.cast()) {
            Ok(mask) => Some(mask),
            Err(error) => {
                set_errno(error);
                return -1;
            }
        }
    };
    select_inner(
        &process,
        descriptor_count,
        read_set,
        write_set,
        error_set,
        None,
        timespec,
        signal_mask
            .as_ref()
            .map_or(core::ptr::null(), |mask| mask as *const CSignalSet),
    )
}