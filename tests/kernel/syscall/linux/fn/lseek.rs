/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{seek_from, Lseek};
use crate::kernel::error::Error;
use crate::kernel::vfs::SeekFrom;

#[cfg(target_arch = "x86_64")]
const EXPECTED_LSEEK: u64 = 8;
#[cfg(target_arch = "aarch64")]
const EXPECTED_LSEEK: u64 = 62;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(Lseek::ID, EXPECTED_LSEEK);
}

#[test]
fn seek_origins_map_to_vfs_positions() {
    assert_eq!(seek_from(12, 0), Ok(SeekFrom::Start(12)));
    assert_eq!(seek_from(-3, 1), Ok(SeekFrom::Current(-3)));
    assert_eq!(seek_from(5, 2), Ok(SeekFrom::End(5)));
}

#[test]
fn seek_rejects_negative_start_and_unknown_origin() {
    assert_eq!(seek_from(-1, 0), Err(Error::Inval));
    assert_eq!(seek_from(0, 3), Err(Error::Inval));
}