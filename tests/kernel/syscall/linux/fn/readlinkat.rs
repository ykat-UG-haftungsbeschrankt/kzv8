/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{output_length, ReadlinkAt};

#[cfg(target_arch = "x86_64")]
const EXPECTED_READLINKAT: u64 = 267;
#[cfg(target_arch = "aarch64")]
const EXPECTED_READLINKAT: u64 = 78;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(ReadlinkAt::ID, EXPECTED_READLINKAT);
}

#[test]
fn readlink_output_is_truncated_to_buffer_without_nul_terminator() {
    assert_eq!(output_length(12, 4), 4);
    assert_eq!(output_length(4, 12), 4);
    assert_eq!(output_length(12, 0), 0);
}