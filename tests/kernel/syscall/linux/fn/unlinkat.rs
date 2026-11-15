/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{validate_flags, UnlinkAt};
use crate::kernel::error::Error;

#[cfg(target_arch = "x86_64")]
const EXPECTED_UNLINKAT: u64 = 263;
#[cfg(target_arch = "aarch64")]
const EXPECTED_UNLINKAT: u64 = 35;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(UnlinkAt::ID, EXPECTED_UNLINKAT);
}

#[test]
fn unlinkat_only_accepts_supported_file_unlink_flags() {
    assert_eq!(validate_flags(0), Ok(()));
    assert_eq!(validate_flags(0x200), Err(Error::NoSys));
    assert_eq!(validate_flags(1), Err(Error::Inval));
}