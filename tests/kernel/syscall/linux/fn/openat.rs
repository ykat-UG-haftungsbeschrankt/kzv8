/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{descriptor_is_allowed, validate_flags, OpenAt};
use crate::kernel::error::Error;

#[cfg(target_arch = "x86_64")]
const EXPECTED_OPENAT: u64 = 257;
#[cfg(target_arch = "aarch64")]
const EXPECTED_OPENAT: u64 = 56;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(OpenAt::ID, EXPECTED_OPENAT);
}

#[test]
fn openat_distinguishes_unsupported_known_flags_from_unknown_bits() {
    assert_eq!(validate_flags(0), Ok(()));
    assert_eq!(validate_flags(0o100), Err(Error::NoSys));
    assert_eq!(validate_flags(0o4010000), Err(Error::NoSys));
    assert_eq!(validate_flags(0o20200000), Err(Error::NoSys));
    assert_eq!(validate_flags(1 << 30), Err(Error::Inval));
}

#[test]
fn openat_enforces_the_descriptor_soft_limit() {
    assert!(descriptor_is_allowed(2, 3));
    assert!(!descriptor_is_allowed(3, 3));
}