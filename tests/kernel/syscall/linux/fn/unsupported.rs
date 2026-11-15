/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::handle;

#[test]
fn missing_syscall_functionality_returns_linux_enosys() {
    let error = handle().unwrap_err();
    assert_eq!(error, crate::kernel::error::Error::NoSys);
    assert_eq!(super::super::super::error::to_syscall_result(error), -38);
}