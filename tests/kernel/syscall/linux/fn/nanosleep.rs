/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{duration_ns, timespec_bytes, Nanosleep};
use crate::kernel::error::Error;

#[cfg(target_arch = "x86_64")]
const EXPECTED_NANOSLEEP: u64 = 35;
#[cfg(target_arch = "aarch64")]
const EXPECTED_NANOSLEEP: u64 = 101;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(Nanosleep::ID, EXPECTED_NANOSLEEP);
}

#[test]
fn validates_and_converts_timespec_duration() {
    assert_eq!(duration_ns(2, 3), Ok(2_000_000_003));
    assert_eq!(duration_ns(-1, 0), Err(Error::Inval));
    assert_eq!(duration_ns(0, 1_000_000_000), Err(Error::Inval));
}

#[test]
fn remaining_time_is_encoded_as_native_timespec() {
    let output = timespec_bytes(2_000_000_003);
    assert_eq!(i64::from_ne_bytes(output[..8].try_into().unwrap()), 2);
    assert_eq!(i64::from_ne_bytes(output[8..].try_into().unwrap()), 3);
}