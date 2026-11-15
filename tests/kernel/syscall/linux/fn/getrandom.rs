/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{requested_length, Getrandom, MAX_GETRANDOM_LENGTH};

#[cfg(target_arch = "x86_64")]
const EXPECTED_GETRANDOM: u64 = 318;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETRANDOM: u64 = 278;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(Getrandom::ID, EXPECTED_GETRANDOM);
}

#[test]
fn random_requests_are_capped_at_linux_batch_limit() {
    assert_eq!(requested_length(0), 0);
    assert_eq!(requested_length(1024), 1024);
    assert_eq!(requested_length(usize::MAX), MAX_GETRANDOM_LENGTH);
}