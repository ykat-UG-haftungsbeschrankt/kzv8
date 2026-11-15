/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{decode_operation, Futex};
use crate::kernel::error::Error;

#[cfg(target_arch = "x86_64")]
const EXPECTED_FUTEX: u64 = 202;
#[cfg(target_arch = "aarch64")]
const EXPECTED_FUTEX: u64 = 98;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(Futex::ID, EXPECTED_FUTEX);
}

#[test]
fn decodes_private_wait_and_wake_operations() {
    assert_eq!(decode_operation(0), Ok(0));
    assert_eq!(decode_operation(1 | 0x80), Ok(1));
    assert_eq!(decode_operation(9 | 0x80 | 0x100), Ok(9));
    assert_eq!(decode_operation(0x1000), Err(Error::Inval));
}