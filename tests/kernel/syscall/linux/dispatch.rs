/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{dispatch_call, SyscallFrame};

const UNIMPLEMENTED_CLOSE_RANGE: u64 = 436;

#[test]
fn known_unimplemented_syscall_returns_linux_enosys() {
    let error = dispatch_call(SyscallFrame {
        number: UNIMPLEMENTED_CLOSE_RANGE,
        arguments: [0; 6],
        windows_arguments: [0; 4],
        result: 0,
        errno: 0,
    })
    .unwrap_err();

    assert_eq!(super::error::to_syscall_result(error), -38);
}