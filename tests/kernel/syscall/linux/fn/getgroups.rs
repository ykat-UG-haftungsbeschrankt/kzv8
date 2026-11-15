/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{decode_groups, encode_groups, group_buffer_size, group_count_result, validate_group_count, GetGroups, SetGroups};
use crate::kernel::error::Error;

#[cfg(target_arch = "x86_64")]
const EXPECTED_GETGROUPS: u64 = 115;
#[cfg(target_arch = "x86_64")]
const EXPECTED_SETGROUPS: u64 = 116;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETGROUPS: u64 = 158;
#[cfg(target_arch = "aarch64")]
const EXPECTED_SETGROUPS: u64 = 159;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(GetGroups::ID, EXPECTED_GETGROUPS);
    assert_eq!(SetGroups::ID, EXPECTED_SETGROUPS);
}

#[test]
fn size_zero_returns_group_count_without_a_buffer() {
    assert_eq!(group_buffer_size(0), Ok(0));
    assert_eq!(group_buffer_size(u64::from(u32::MAX)), Err(Error::Inval));
    assert_eq!(group_buffer_size(u64::MAX), Err(Error::Inval));
    assert_eq!(group_count_result(0, 3), Ok(Some(3)));
}

#[test]
fn undersized_group_buffer_is_rejected() {
    assert_eq!(group_count_result(2, 3), Err(Error::Inval));
    assert_eq!(group_count_result(3, 3), Ok(None));
}

#[test]
fn group_ids_are_encoded_in_native_endian_order() {
    let expected = [7u32.to_ne_bytes(), 0x1234_5678u32.to_ne_bytes()].concat();
    assert_eq!(encode_groups(&[7, 0x1234_5678]), Ok(expected));
}

#[test]
fn setgroups_rejects_counts_above_the_kernel_limit() {
    assert_eq!(validate_group_count(65_536), Ok(()));
    assert_eq!(validate_group_count(65_537), Err(Error::Inval));
}

#[test]
fn setgroups_decodes_native_endian_ids() {
    let bytes = [7u32.to_ne_bytes(), 0x1234_5678u32.to_ne_bytes()].concat();
    assert_eq!(decode_groups(&bytes), Ok(vec![7, 0x1234_5678]));
}