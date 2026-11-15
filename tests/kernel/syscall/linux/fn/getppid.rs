/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::GetPpid;

#[cfg(target_arch = "x86_64")]
const EXPECTED_GETPPID: u64 = 110;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETPPID: u64 = 173;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(GetPpid::ID, EXPECTED_GETPPID);
}