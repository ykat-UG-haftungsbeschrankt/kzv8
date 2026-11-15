/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{validate_linkat_flags, LinkAt, SymlinkAt};
use crate::kernel::error::Error;

#[cfg(target_arch = "x86_64")]
const EXPECTED_LINKAT: u64 = 265;
#[cfg(target_arch = "x86_64")]
const EXPECTED_SYMLINKAT: u64 = 266;
#[cfg(target_arch = "aarch64")]
const EXPECTED_LINKAT: u64 = 37;
#[cfg(target_arch = "aarch64")]
const EXPECTED_SYMLINKAT: u64 = 36;

#[test]
fn syscall_ids_match_linux_architecture_abi() {
    assert_eq!(LinkAt::ID, EXPECTED_LINKAT);
    assert_eq!(SymlinkAt::ID, EXPECTED_SYMLINKAT);
}

#[test]
fn linkat_rejects_flags_without_implemented_semantics() {
    assert_eq!(validate_linkat_flags(0), Ok(()));
    assert_eq!(validate_linkat_flags(0x400), Err(Error::NoSys));
    assert_eq!(validate_linkat_flags(0x1000), Err(Error::NoSys));
    assert_eq!(validate_linkat_flags(1), Err(Error::Inval));
}