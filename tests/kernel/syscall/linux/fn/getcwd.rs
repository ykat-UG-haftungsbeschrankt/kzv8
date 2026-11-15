/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{build_buffer, GetCwd};
use crate::kernel::error::Error;

#[cfg(target_arch = "x86_64")]
const EXPECTED_GETCWD: u64 = 79;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETCWD: u64 = 17;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(GetCwd::ID, EXPECTED_GETCWD);
}

#[test]
fn getcwd_buffer_includes_a_trailing_nul() {
    assert_eq!(build_buffer(b"/tmp", 5), Ok(b"/tmp\0".to_vec()));
}

#[test]
fn getcwd_rejects_zero_or_insufficient_size() {
    assert_eq!(build_buffer(b"/", 0), Err(Error::Inval));
    assert_eq!(build_buffer(b"/", 1), Err(Error::Range));
}