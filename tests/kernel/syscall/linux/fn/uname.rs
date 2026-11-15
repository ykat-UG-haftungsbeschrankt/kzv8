/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{utsname_bytes, Uname};

#[cfg(target_arch = "x86_64")]
const EXPECTED_UNAME: u64 = 63;
#[cfg(target_arch = "aarch64")]
const EXPECTED_UNAME: u64 = 160;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(Uname::ID, EXPECTED_UNAME);
}

#[test]
fn utsname_fields_are_fixed_size_and_nul_terminated() {
    let output = utsname_bytes();
    assert_eq!(output.len(), 390);
    assert_eq!(&output[..6], b"Linux\0");
    assert_eq!(output[64], 0);
    assert_eq!(output[129], 0);
    assert_eq!(output[194], 0);
    assert_eq!(output[259], 0);
    assert_eq!(output[324], 0);
    assert_eq!(output[389], 0);
}