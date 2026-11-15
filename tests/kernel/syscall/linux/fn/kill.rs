/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::Kill;

#[cfg(target_arch = "x86_64")]
const EXPECTED_KILL: u64 = 62;
#[cfg(target_arch = "aarch64")]
const EXPECTED_KILL: u64 = 129;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(Kill::ID, EXPECTED_KILL);
}