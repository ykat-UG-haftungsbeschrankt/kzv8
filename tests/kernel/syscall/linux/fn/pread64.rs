/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{validate_arguments, Pread64};
use crate::kernel::error::Error;

#[cfg(target_arch = "x86_64")]
const EXPECTED_PREAD64: u64 = 17;
#[cfg(target_arch = "aarch64")]
const EXPECTED_PREAD64: u64 = 67;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(Pread64::ID, EXPECTED_PREAD64);
}

#[test]
fn positional_read_rejects_negative_offset_and_oversized_count() {
    assert_eq!(validate_arguments(16, 0), Ok(()));
    assert_eq!(validate_arguments(16, i64::MAX as u64 + 1), Err(Error::Inval));
    assert_eq!(validate_arguments(1 << 20, 0), Ok(()));
    assert_eq!(validate_arguments((1 << 20) + 1, 0), Err(Error::Fbig));
}