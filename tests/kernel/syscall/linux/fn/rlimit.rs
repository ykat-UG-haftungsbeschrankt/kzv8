/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{decode_limits, encode_limits, kernel_resource, Getrlimit, Setrlimit};
use crate::kernel::error::Error;

#[cfg(target_arch = "x86_64")]
const EXPECTED_GETRLIMIT: u64 = 97;
#[cfg(target_arch = "x86_64")]
const EXPECTED_SETRLIMIT: u64 = 160;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETRLIMIT: u64 = 163;
#[cfg(target_arch = "aarch64")]
const EXPECTED_SETRLIMIT: u64 = 164;

#[test]
fn syscall_ids_match_linux_architecture_abi() {
    assert_eq!(Getrlimit::ID, EXPECTED_GETRLIMIT);
    assert_eq!(Setrlimit::ID, EXPECTED_SETRLIMIT);
}

#[test]
fn resource_ids_map_to_kernel_slots_without_aliasing_unsupported_limits() {
    assert_eq!(kernel_resource(0), Ok(0));
    assert_eq!(kernel_resource(7), Ok(5));
    assert_eq!(kernel_resource(9), Ok(9));
    assert_eq!(kernel_resource(5), Err(Error::NoSys));
    assert_eq!(kernel_resource(16), Err(Error::Inval));
}

#[test]
fn rlimit_pair_uses_linux_native_64_bit_layout() {
    let encoded = encode_limits(1024, u64::MAX);
    assert_eq!(decode_limits(&encoded), (1024, u64::MAX));
}