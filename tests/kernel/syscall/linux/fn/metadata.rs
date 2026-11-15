/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{chmod_mode, Chmod, Chown, Fchmod, Fchown};

#[cfg(target_arch = "x86_64")]
const EXPECTED_CHMOD: u64 = 90;
#[cfg(target_arch = "x86_64")]
const EXPECTED_FCHMOD: u64 = 91;
#[cfg(target_arch = "x86_64")]
const EXPECTED_CHOWN: u64 = 92;
#[cfg(target_arch = "x86_64")]
const EXPECTED_FCHOWN: u64 = 93;
#[cfg(target_arch = "aarch64")]
const EXPECTED_CHMOD: u64 = 53;
#[cfg(target_arch = "aarch64")]
const EXPECTED_FCHMOD: u64 = 52;
#[cfg(target_arch = "aarch64")]
const EXPECTED_CHOWN: u64 = 54;
#[cfg(target_arch = "aarch64")]
const EXPECTED_FCHOWN: u64 = 55;

#[test]
fn syscall_ids_match_linux_architecture_abi() {
    assert_eq!(Chmod::ID, EXPECTED_CHMOD);
    assert_eq!(Fchmod::ID, EXPECTED_FCHMOD);
    assert_eq!(Chown::ID, EXPECTED_CHOWN);
    assert_eq!(Fchown::ID, EXPECTED_FCHOWN);
}

#[test]
fn chmod_ignores_file_type_bits_and_preserves_special_mode_bits() {
    assert_eq!(chmod_mode(0o1004755), 0o4755);
}