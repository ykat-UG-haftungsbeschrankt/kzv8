/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::to_syscall_result;
use crate::kernel::error::Error;

#[test]
fn linux_errno_values_are_encoded_as_negative_results() {
    assert_eq!(to_syscall_result(Error::NoSys), -38);
    assert_eq!(to_syscall_result(Error::Perm), -1);
    assert_eq!(to_syscall_result(Error::WouldBlock), -11);
    assert_eq!(to_syscall_result(Error::Deadlock), -35);
    assert_eq!(to_syscall_result(Error::Restart), -4);
}

#[test]
fn invalid_error_sentinels_do_not_escape_to_userspace() {
    assert_eq!(to_syscall_result(Error::Min), -22);
    assert_eq!(to_syscall_result(Error::Max), -22);
}