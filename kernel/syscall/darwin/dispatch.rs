/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::error;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::syscall::SyscallFrame;

const SYSCALL_CLASS_MASK: u64 = 0x0f00_0000;
const SYSCALL_CLASS_UNIX: u64 = 0x0200_0000;
const SYSCALL_NUMBER_MASK: u64 = 0x00ff_ffff;

const SYS_EXIT: u64 = 1;
const SYS_READ: u64 = 3;
const SYS_WRITE: u64 = 4;
const SYS_OPEN: u64 = 5;
const SYS_CLOSE: u64 = 6;
const SYS_GETPID: u64 = 20;
const SYS_SETUID: u64 = 23;
const SYS_GETUID: u64 = 24;
const SYS_GETEUID: u64 = 25;
const SYS_KILL: u64 = 37;
const SYS_GETPPID: u64 = 39;
const SYS_GETEGID: u64 = 43;
const SYS_GETGID: u64 = 47;
const SYS_UMASK: u64 = 60;
const SYS_MUNMAP: u64 = 73;
const SYS_MPROTECT: u64 = 74;
const SYS_GETGROUPS: u64 = 79;
const SYS_SETGROUPS: u64 = 80;
const SYS_GETPGRP: u64 = 81;
const SYS_SETPGID: u64 = 82;
const SYS_SETREUID: u64 = 126;
const SYS_SETREGID: u64 = 127;
const SYS_SETSID: u64 = 147;
const SYS_GETPGID: u64 = 151;
const SYS_SETGID: u64 = 181;
const SYS_SETEGID: u64 = 182;
const SYS_SETEUID: u64 = 183;
const SYS_MMAP: u64 = 197;
const SYS_LSEEK: u64 = 199;
const SYS_GETSID: u64 = 310;
const SYS_GETCWD: u64 = 326;
const SYS_THREAD_SELFID: u64 = 372;
const SYS_OPENAT: u64 = 463;

const DARWIN_AT_FDCWD: i64 = -2;
const LINUX_AT_FDCWD: i64 = -100;

pub(crate) const fn has_syscall_class(number: u64) -> bool {
    number & SYSCALL_CLASS_MASK != 0
}

pub(crate) const fn with_unix_class(number: u64) -> u64 {
    if has_syscall_class(number) {
        number
    } else {
        number | SYSCALL_CLASS_UNIX
    }
}

fn decode_syscall_number(number: u64) -> Result<u64, Error> {
    if number & SYSCALL_CLASS_MASK != SYSCALL_CLASS_UNIX {
        return Err(Error::NoSys);
    }
    Ok(number & SYSCALL_NUMBER_MASK)
}

fn dispatch_call(frame: SyscallFrame) -> Result<i64, Error> {
    let number = decode_syscall_number(frame.number)?;
    match number {
        SYS_SETEUID => {
            ThreadManager::current_process()?.set_effective_user(frame.arguments[0] as u32)?;
            Ok(0)
        }
        SYS_SETEGID => {
            ThreadManager::current_process()?.set_effective_group(frame.arguments[0] as u32)?;
            Ok(0)
        }
        _ => crate::kernel::syscall::linux::dispatch_call(translate_call(number, frame)?),
    }
}

fn translate_syscall(frame: SyscallFrame) -> Result<SyscallFrame, Error> {
    translate_call(decode_syscall_number(frame.number)?, frame)
}

fn translate_call(number: u64, frame: SyscallFrame) -> Result<SyscallFrame, Error> {
    let mut arguments = frame.arguments;
    let linux_number = match number {
        SYS_EXIT => crate::kernel::arch::syscall::ExitGroup,
        SYS_READ => crate::kernel::arch::syscall::Read,
        SYS_WRITE => crate::kernel::arch::syscall::Write,
        SYS_OPEN => {
            arguments = [
                LINUX_AT_FDCWD as u64,
                frame.arguments[0],
                translate_open_flags(frame.arguments[1])?,
                frame.arguments[2],
                0,
                0,
            ];
            crate::kernel::arch::syscall::OpenAt
        }
        SYS_CLOSE => crate::kernel::arch::syscall::Close,
        SYS_GETPID => crate::kernel::arch::syscall::GetPid,
        SYS_SETUID => crate::kernel::arch::syscall::Setuid,
        SYS_GETUID => crate::kernel::arch::syscall::GetUid,
        SYS_GETEUID => crate::kernel::arch::syscall::GetEuid,
        SYS_KILL => crate::kernel::arch::syscall::Kill,
        SYS_GETPPID => crate::kernel::arch::syscall::GetPpid,
        SYS_GETEGID => crate::kernel::arch::syscall::GetEgid,
        SYS_GETGID => crate::kernel::arch::syscall::GetGid,
        SYS_UMASK => crate::kernel::arch::syscall::Umask,
        SYS_MUNMAP => crate::kernel::arch::syscall::Munmap,
        SYS_MPROTECT => crate::kernel::arch::syscall::Mprotect,
        SYS_GETGROUPS => crate::kernel::arch::syscall::GetGroups,
        SYS_SETGROUPS => crate::kernel::arch::syscall::SetGroups,
        SYS_GETPGRP => {
            arguments[0] = 0;
            crate::kernel::arch::syscall::Getpgid
        }
        SYS_SETPGID => crate::kernel::arch::syscall::Setpgid,
        SYS_SETREUID => crate::kernel::arch::syscall::Setreuid,
        SYS_SETREGID => crate::kernel::arch::syscall::Setregid,
        SYS_SETSID => crate::kernel::arch::syscall::Setsid,
        SYS_GETPGID => crate::kernel::arch::syscall::Getpgid,
        SYS_SETGID => crate::kernel::arch::syscall::Setgid,
        SYS_MMAP => {
            arguments[3] = translate_mmap_flags(arguments[3])?;
            crate::kernel::arch::syscall::Mmap
        }
        SYS_LSEEK => crate::kernel::arch::syscall::Lseek,
        SYS_GETSID => crate::kernel::arch::syscall::Getsid,
        SYS_GETCWD => crate::kernel::arch::syscall::GetCwd,
        SYS_THREAD_SELFID => crate::kernel::arch::syscall::GetTid,
        SYS_OPENAT => {
            if arguments[0] as i64 == DARWIN_AT_FDCWD {
                arguments[0] = LINUX_AT_FDCWD as u64;
            }
            arguments[2] = translate_open_flags(arguments[2])?;
            crate::kernel::arch::syscall::OpenAt
        }
        _ => return Err(Error::NoSys),
    };

    Ok(SyscallFrame {
        number: linux_number,
        arguments,
        windows_arguments: [0; 4],
        result: 0,
        errno: 0,
    })
}

fn translate_open_flags(flags: u64) -> Result<u64, Error> {
    const ACCESS_MODE_MASK: u64 = 0x3;
    const DIRECTORY: u64 = 0x0010_0000;
    const CLOEXEC: u64 = 0x0100_0000;
    const SUPPORTED: u64 = ACCESS_MODE_MASK | DIRECTORY | CLOEXEC;
    const LINUX_DIRECTORY: u64 = 0x1_0000;
    const LINUX_CLOEXEC: u64 = 0x8_0000;

    if flags & !SUPPORTED != 0 {
        return Err(Error::Inval);
    }
    let mut translated = flags & ACCESS_MODE_MASK;
    if flags & DIRECTORY != 0 {
        translated |= LINUX_DIRECTORY;
    }
    if flags & CLOEXEC != 0 {
        translated |= LINUX_CLOEXEC;
    }
    Ok(translated)
}

fn translate_mmap_flags(flags: u64) -> Result<u64, Error> {
    const DARWIN_MAP_PRIVATE: u64 = 0x2;
    const DARWIN_MAP_NORESERVE: u64 = 0x40;
    const DARWIN_MAP_ANON: u64 = 0x1000;
    const SUPPORTED: u64 = DARWIN_MAP_PRIVATE | DARWIN_MAP_NORESERVE | DARWIN_MAP_ANON;
    const LINUX_MAP_PRIVATE: u64 = 0x2;
    const LINUX_MAP_ANONYMOUS: u64 = 0x20;
    const LINUX_MAP_NORESERVE: u64 = 0x4000;

    if flags & !SUPPORTED != 0
        || flags & (DARWIN_MAP_PRIVATE | DARWIN_MAP_ANON)
            != (DARWIN_MAP_PRIVATE | DARWIN_MAP_ANON)
    {
        return Err(Error::Inval);
    }
    let mut translated = LINUX_MAP_PRIVATE | LINUX_MAP_ANONYMOUS;
    if flags & DARWIN_MAP_NORESERVE != 0 {
        translated |= LINUX_MAP_NORESERVE;
    }
    Ok(translated)
}

pub(crate) fn dispatch(frame: &mut SyscallFrame) {
    ThreadManager::begin_syscall_accounting();
    frame.errno = 0;
    match dispatch_call(*frame) {
        Ok(value) => frame.result = value,
        Err(error) => {
            let errno = error::errno(error);
            frame.result = i64::from(errno);
            frame.errno = errno;
        }
    }
    ThreadManager::end_syscall_accounting();
}

#[cfg(test)]
#[path = "../../../tests/kernel/syscall/darwin/dispatch.rs"]
mod tests;