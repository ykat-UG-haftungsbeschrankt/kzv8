/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_char, c_int};
use core::mem::{size_of, MaybeUninit};

use crate::kernel::dl::AddressSpace;
use crate::kernel::runtime::runtime_libc::{filesystem, network, pty};
use crate::kernel::scheduler::thread::ThreadManager;

use super::errno::{as_errno, EFAULT, EINVAL, ENOSYS};

const O_WRONLY: c_int = 1;
const O_CREAT: c_int = 0o100;
const O_TRUNC: c_int = 0o1000;
const O_NONBLOCK: c_int = 0o4000;
const O_ACCMODE: c_int = 0o3;
const O_APPEND: c_int = 0o2000;
const F_DUPFD: c_int = 0;
const F_GETFD: c_int = 1;
const F_SETFD: c_int = 2;
const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
const F_GETLK: c_int = 5;
const F_SETLK: c_int = 6;
const F_SETLKW: c_int = 7;
const F_DUPFD_CLOEXEC: c_int = 1030;

type FileLockRecord = crate::kernel::runtime::runtime_libc::filesystem::FileLockRecord;

fn read_lock(
    argument: usize,
) -> Result<(FileLockRecord, crate::kernel::scheduler::thread::vm::UserAddressSpace), c_int> {
    if argument == 0 {
        return Err(EFAULT);
    }
    let process = ThreadManager::current_process().map_err(as_errno)?;
    let address_space = process.address_space();
    let mut lock = MaybeUninit::<FileLockRecord>::uninit();
    let bytes = unsafe { core::slice::from_raw_parts_mut(lock.as_mut_ptr().cast::<u8>(), size_of::<FileLockRecord>()) };
    address_space.read(argument, bytes).map_err(as_errno)?;
    Ok((unsafe { lock.assume_init() }, address_space))
}

fn write_lock(
    argument: usize,
    lock: &FileLockRecord,
    address_space: &mut crate::kernel::scheduler::thread::vm::UserAddressSpace,
) -> Result<(), c_int> {
    let bytes = unsafe {
        core::slice::from_raw_parts((lock as *const FileLockRecord).cast::<u8>(), size_of::<FileLockRecord>())
    };
    address_space.write(argument, bytes).map_err(as_errno)
}

unsafe extern "C" {
    fn kz_rust_open(path: *const c_char, flags: c_int, mode: u32) -> c_int;
    fn kz_rust_openat(
        directory_descriptor: c_int,
        path: *const c_char,
        flags: c_int,
        mode: u32,
    ) -> c_int;
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub unsafe extern "C" fn creat(path: *const c_char, mode: u32) -> c_int {
    unsafe { kz_rust_open(path, O_WRONLY | O_CREAT | O_TRUNC, mode) }
}

#[no_mangle]
pub unsafe extern "C" fn fcntl(
    file_descriptor: c_int,
    command: c_int,
    argument: usize,
) -> c_int {
    if let Some(alias) = crate::kernel::runtime::runtime_libc::fd_alias::get(file_descriptor) {
        match command {
            F_GETFD => return c_int::from(alias.close_on_exec),
            F_SETFD => {
                let flags = argument as c_int;
                if flags & !1 != 0 {
                    set_errno(EINVAL);
                    return -1;
                }
                return match crate::kernel::runtime::runtime_libc::fd_alias::set_close_on_exec(
                    file_descriptor,
                    flags & 1 != 0,
                ) {
                    Ok(()) => 0,
                    Err(error) => {
                        set_errno(-(error as c_int));
                        -1
                    }
                };
            }
            _ => {}
        }
    }
    if pty::is_fd(file_descriptor) {
        return match command {
            F_DUPFD | F_DUPFD_CLOEXEC => {
                let Ok(minimum) = c_int::try_from(argument) else {
                    set_errno(EINVAL);
                    return -1;
                };
                pty::duplicate(
                    file_descriptor,
                    minimum,
                    command == F_DUPFD_CLOEXEC,
                )
                .unwrap_or_else(|error| {
                    set_errno(-(error as i32));
                    -1
                })
            }
            F_GETFD => pty::descriptor_flags(file_descriptor).unwrap_or_else(|error| {
                set_errno(-(error as i32));
                -1
            }),
            F_SETFD => match pty::set_descriptor_flags(file_descriptor, argument as c_int) {
                Ok(()) => 0,
                Err(error) => {
                    set_errno(-(error as i32));
                    -1
                }
            },
            F_GETFL => pty::status_flags(file_descriptor).unwrap_or_else(|error| {
                set_errno(-(error as i32));
                -1
            }),
            F_SETFL => match pty::set_status_flags(file_descriptor, argument as c_int) {
                Ok(()) => 0,
                Err(error) => {
                    set_errno(-(error as i32));
                    -1
                }
            },
            _ => {
                set_errno(ENOSYS);
                -1
            }
        };
    }
    if network::is_socket_fd(file_descriptor) {
        return match command {
            F_DUPFD | F_DUPFD_CLOEXEC => {
                let Ok(minimum) = c_int::try_from(argument) else {
                    set_errno(EINVAL);
                    return -1;
                };
                network::duplicate_socket(
                    file_descriptor,
                    minimum,
                    command == F_DUPFD_CLOEXEC,
                )
                .unwrap_or_else(|error| {
                    set_errno(-(error as i32));
                    -1
                })
            }
            F_GETFD => network::descriptor_flags(file_descriptor).unwrap_or_else(|error| {
                set_errno(-(error as i32));
                -1
            }),
            F_SETFD => match network::set_descriptor_flags(file_descriptor, argument as c_int) {
                Ok(()) => 0,
                Err(error) => {
                    set_errno(-(error as i32));
                    -1
                }
            },
            F_GETFL => network::get_status_flags(file_descriptor).unwrap_or_else(|error| {
                set_errno(-(error as i32));
                -1
            }),
            F_SETFL => {
                if argument as c_int & !(O_NONBLOCK | O_ACCMODE) != 0 {
                    set_errno(EINVAL);
                    return -1;
                }
                match network::set_status_flags(file_descriptor, argument as c_int) {
                    Ok(()) => 0,
                    Err(error) => {
                        set_errno(-(error as i32));
                        -1
                    }
                }
            }
            _ => {
                set_errno(ENOSYS);
                -1
            }
        };
    }
    if network::is_packet_fd(file_descriptor) {
        return match command {
            F_DUPFD | F_DUPFD_CLOEXEC => {
                let Ok(minimum) = c_int::try_from(argument) else {
                    set_errno(EINVAL);
                    return -1;
                };
                network::duplicate_packet_fd(
                    file_descriptor,
                    minimum,
                    command == F_DUPFD_CLOEXEC,
                )
                .unwrap_or_else(|error| {
                    set_errno(-(error as i32));
                    -1
                })
            }
            F_GETFD => network::packet_descriptor_flags(file_descriptor).unwrap_or_else(|error| {
                set_errno(-(error as i32));
                -1
            }),
            F_SETFD => match network::set_packet_descriptor_flags(file_descriptor, argument as c_int) {
                Ok(()) => 0,
                Err(error) => {
                    set_errno(-(error as i32));
                    -1
                }
            },
            F_GETFL => network::get_status_flags(file_descriptor).unwrap_or_else(|error| {
                set_errno(-(error as i32));
                -1
            }),
            F_SETFL => {
                let flags = argument as c_int;
                if flags & !(O_NONBLOCK | O_ACCMODE) != 0 {
                    set_errno(EINVAL);
                    return -1;
                }
                match network::set_status_flags(file_descriptor, flags) {
                    Ok(()) => 0,
                    Err(error) => {
                        set_errno(-(error as i32));
                        -1
                    }
                }
            }
            _ => {
                set_errno(ENOSYS);
                -1
            }
        };
    }
    match command {
        F_GETLK => {
            let (mut lock, mut address_space) = match read_lock(argument) {
                Ok(lock) => lock,
                Err(error) => {
                    set_errno(error);
                    return -1;
                }
            };
            match filesystem::query_record_lock(file_descriptor, &mut lock)
                .and_then(|()| write_lock(argument, &lock, &mut address_space))
            {
                Ok(()) => 0,
                Err(error) => {
                    set_errno(error);
                    -1
                }
            }
        }
        F_SETLK | F_SETLKW => {
            let (lock, _) = match read_lock(argument) {
                Ok(lock) => lock,
                Err(error) => {
                    set_errno(error);
                    return -1;
                }
            };
            match filesystem::set_record_lock(file_descriptor, &lock, command == F_SETLKW) {
                Ok(()) => 0,
                Err(error) => {
                    set_errno(error);
                    -1
                }
            }
        }
        F_DUPFD | F_DUPFD_CLOEXEC => {
            let Ok(minimum) = c_int::try_from(argument) else {
                set_errno(EINVAL);
                return -1;
            };
            match filesystem::duplicate(
                file_descriptor,
                minimum,
                command == F_DUPFD_CLOEXEC,
            ) {
                Ok(duplicate) => duplicate,
                Err(error) => {
                    set_errno(error);
                    -1
                }
            }
        }
        F_GETFD => match filesystem::descriptor_flags(file_descriptor) {
            Ok(flags) => flags,
            Err(error) => {
                set_errno(error);
                -1
            }
        },
        F_SETFD => match filesystem::set_descriptor_flags(file_descriptor, argument as c_int) {
            Ok(()) => 0,
            Err(error) => {
                set_errno(error);
                -1
            }
        },
        F_GETFL => match filesystem::status_flags(file_descriptor) {
            Ok(flags) => flags,
            Err(error) => {
                set_errno(error);
                -1
            }
        },
        F_SETFL => {
            let flags = argument as c_int;
            if flags & !(O_NONBLOCK | O_APPEND | O_ACCMODE) != 0 {
                set_errno(EINVAL);
                return -1;
            }
            match filesystem::set_status_flags(file_descriptor, flags) {
                Ok(()) => 0,
                Err(error) => {
                    set_errno(error);
                    -1
                }
            }
        }
        _ => {
            set_errno(ENOSYS);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn openat(
    directory_descriptor: c_int,
    path: *const c_char,
    flags: c_int,
    mode: u32,
) -> c_int {
    unsafe { kz_rust_openat(directory_descriptor, path, flags, mode) }
}

#[no_mangle]
pub extern "C" fn posix_fadvise(
    _file_descriptor: c_int,
    _offset: i64,
    _length: i64,
    _advice: c_int,
) -> c_int {
    ENOSYS
}

#[no_mangle]
pub extern "C" fn posix_fallocate(
    file_descriptor: c_int,
    offset: i64,
    length: i64,
) -> c_int {
    if crate::kernel::runtime::runtime_libc::network::is_socket_fd(file_descriptor)
        || crate::kernel::runtime::runtime_libc::network::is_packet_fd(file_descriptor)
    {
        return EINVAL;
    }
    match filesystem::posix_fallocate(file_descriptor, offset, length) {
        Ok(()) => 0,
        Err(error) => error,
    }
}