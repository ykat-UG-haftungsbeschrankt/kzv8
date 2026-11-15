/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{
    validate_mapping_mode, Mmap, MAP_ANONYMOUS, MAP_DENYWRITE, MAP_EXECUTABLE, MAP_FIXED,
    MAP_FIXED_NOREPLACE, MAP_PRIVATE, MAP_SHARED,
};
use crate::kernel::error::Error;

#[cfg(target_arch = "x86_64")]
const EXPECTED_MMAP: u64 = 9;
#[cfg(target_arch = "aarch64")]
const EXPECTED_MMAP: u64 = 222;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(Mmap::ID, EXPECTED_MMAP);
}

#[test]
fn mmap_distinguishes_supported_unsupported_and_invalid_modes() {
    assert_eq!(validate_mapping_mode(MAP_PRIVATE | MAP_ANONYMOUS, 0), Ok(()));
    assert_eq!(validate_mapping_mode(MAP_SHARED | MAP_ANONYMOUS, 0), Err(Error::NoSys));
    assert_eq!(validate_mapping_mode(MAP_PRIVATE, 0), Ok(()));
    assert_eq!(
        validate_mapping_mode(MAP_PRIVATE | MAP_FIXED | MAP_DENYWRITE | MAP_EXECUTABLE, 0),
        Ok(())
    );
    assert_eq!(validate_mapping_mode(MAP_PRIVATE | MAP_FIXED_NOREPLACE, 0), Ok(()));
    assert_eq!(validate_mapping_mode(MAP_PRIVATE | MAP_ANONYMOUS | 0x8000_0000, 0), Err(Error::Inval));
    assert_eq!(validate_mapping_mode(MAP_PRIVATE | MAP_ANONYMOUS, 1), Err(Error::Inval));
}