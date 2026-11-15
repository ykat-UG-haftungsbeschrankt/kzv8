/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{linux_priority, Getpriority, Setpriority};

#[cfg(target_arch = "x86_64")]
const EXPECTED_GETPRIORITY: u64 = 140;
#[cfg(target_arch = "x86_64")]
const EXPECTED_SETPRIORITY: u64 = 141;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETPRIORITY: u64 = 141;
#[cfg(target_arch = "aarch64")]
const EXPECTED_SETPRIORITY: u64 = 140;

#[test]
fn syscall_ids_match_linux_architecture_abi() {
    assert_eq!(Getpriority::ID, EXPECTED_GETPRIORITY);
    assert_eq!(Setpriority::ID, EXPECTED_SETPRIORITY);
}

#[test]
fn getpriority_uses_linux_positive_return_range() {
    assert_eq!(linux_priority(-20), 40);
    assert_eq!(linux_priority(0), 20);
    assert_eq!(linux_priority(19), 1);
}