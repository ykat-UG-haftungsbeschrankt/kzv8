/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{
    has_syscall_class, translate_mmap_flags, translate_open_flags, translate_syscall,
    with_unix_class, SYS_GETPGRP, SYS_OPEN, SYS_READ, SYSCALL_CLASS_UNIX,
};
use crate::kernel::error::Error;
use crate::kernel::syscall::SyscallFrame;

fn frame(number: u64, arguments: [u64; 6]) -> SyscallFrame {
    SyscallFrame {
        number,
        arguments,
        windows_arguments: [0; 4],
        result: 0,
        errno: 0,
    }
}

#[test]
fn adds_unix_class_to_unprefixed_arm64_syscalls() {
    assert_eq!(with_unix_class(SYS_READ), SYSCALL_CLASS_UNIX | SYS_READ);
    assert_eq!(with_unix_class(SYSCALL_CLASS_UNIX | SYS_READ), SYSCALL_CLASS_UNIX | SYS_READ);
    assert!(has_syscall_class(SYSCALL_CLASS_UNIX | SYS_READ));
}

#[test]
fn translates_read_to_the_existing_kernel_handler() {
    let arguments = [3, 0x1000, 128, 0, 0, 0];
    let translated = translate_syscall(frame(SYSCALL_CLASS_UNIX | SYS_READ, arguments)).unwrap();

    assert_eq!(translated.number, crate::kernel::arch::syscall::Read);
    assert_eq!(translated.arguments, arguments);
}

#[test]
fn translates_open_flags_and_at_fdcwd() {
    const DARWIN_O_DIRECTORY: u64 = 0x0010_0000;
    const DARWIN_O_CLOEXEC: u64 = 0x0100_0000;

    let flags = DARWIN_O_DIRECTORY | DARWIN_O_CLOEXEC;
    let translated = translate_syscall(frame(
        SYSCALL_CLASS_UNIX | SYS_OPEN,
        [0x1000, flags, 0, 0, 0, 0],
    ))
    .unwrap();

    assert_eq!(translated.number, crate::kernel::arch::syscall::OpenAt);
    assert_eq!(translated.arguments[0] as i64, -100);
    assert_eq!(translated.arguments[1], 0x1000);
    assert_eq!(translated.arguments[2], 0x1_0000 | 0x8_0000);
}

#[test]
fn translates_getpgrp_to_the_current_process_group() {
    let translated = translate_syscall(frame(
        SYSCALL_CLASS_UNIX | SYS_GETPGRP,
        [u64::MAX; 6],
    ))
    .unwrap();

    assert_eq!(translated.number, crate::kernel::arch::syscall::Getpgid);
    assert_eq!(translated.arguments[0], 0);
}

#[test]
fn translates_supported_mmap_flags() {
    let flags = 0x2 | 0x40 | 0x1000;
    assert_eq!(translate_mmap_flags(flags), Ok(0x2 | 0x20 | 0x4000));
    assert_eq!(translate_mmap_flags(0x1 | 0x1000), Err(Error::Inval));
}

#[test]
fn rejects_non_unix_and_unimplemented_calls() {
    assert_eq!(
        translate_syscall(frame(0x0100_0000 | SYS_READ, [0; 6])),
        Err(Error::NoSys),
    );
    assert_eq!(
        translate_syscall(frame(SYSCALL_CLASS_UNIX | 999, [0; 6])),
        Err(Error::NoSys),
    );
    assert_eq!(translate_open_flags(0x200), Err(Error::Inval));
    assert_eq!(translate_mmap_flags(0x2 | 0x1000 | 0x10), Err(Error::Inval));
}