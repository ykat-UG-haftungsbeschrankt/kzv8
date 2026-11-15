/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{
    optional_id, GetEgid, GetEuid, GetGid, GetResGid, GetResUid, GetUid, SetGid, SetReGid,
    SetReUid, SetResGid, SetResUid, SetUid,
};

#[cfg(target_arch = "x86_64")]
const EXPECTED_GETUID: u64 = 102;
#[cfg(target_arch = "x86_64")]
const EXPECTED_GETEUID: u64 = 107;
#[cfg(target_arch = "x86_64")]
const EXPECTED_GETGID: u64 = 104;
#[cfg(target_arch = "x86_64")]
const EXPECTED_GETEGID: u64 = 108;
#[cfg(target_arch = "x86_64")]
const EXPECTED_GETRESUID: u64 = 118;
#[cfg(target_arch = "x86_64")]
const EXPECTED_GETRESGID: u64 = 120;
#[cfg(target_arch = "x86_64")]
const EXPECTED_SETUID: u64 = 105;
#[cfg(target_arch = "x86_64")]
const EXPECTED_SETGID: u64 = 106;
#[cfg(target_arch = "x86_64")]
const EXPECTED_SETREUID: u64 = 113;
#[cfg(target_arch = "x86_64")]
const EXPECTED_SETREGID: u64 = 114;
#[cfg(target_arch = "x86_64")]
const EXPECTED_SETRESUID: u64 = 117;
#[cfg(target_arch = "x86_64")]
const EXPECTED_SETRESGID: u64 = 119;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETUID: u64 = 174;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETEUID: u64 = 175;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETGID: u64 = 176;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETEGID: u64 = 177;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETRESUID: u64 = 148;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETRESGID: u64 = 150;
#[cfg(target_arch = "aarch64")]
const EXPECTED_SETUID: u64 = 146;
#[cfg(target_arch = "aarch64")]
const EXPECTED_SETGID: u64 = 144;
#[cfg(target_arch = "aarch64")]
const EXPECTED_SETREUID: u64 = 145;
#[cfg(target_arch = "aarch64")]
const EXPECTED_SETREGID: u64 = 143;
#[cfg(target_arch = "aarch64")]
const EXPECTED_SETRESUID: u64 = 147;
#[cfg(target_arch = "aarch64")]
const EXPECTED_SETRESGID: u64 = 149;

#[test]
fn credential_syscall_ids_match_linux_architecture_abi() {
    assert_eq!(GetUid::ID, EXPECTED_GETUID);
    assert_eq!(GetEuid::ID, EXPECTED_GETEUID);
    assert_eq!(GetGid::ID, EXPECTED_GETGID);
    assert_eq!(GetEgid::ID, EXPECTED_GETEGID);
    assert_eq!(GetResUid::ID, EXPECTED_GETRESUID);
    assert_eq!(GetResGid::ID, EXPECTED_GETRESGID);
    assert_eq!(SetUid::ID, EXPECTED_SETUID);
    assert_eq!(SetGid::ID, EXPECTED_SETGID);
    assert_eq!(SetReUid::ID, EXPECTED_SETREUID);
    assert_eq!(SetReGid::ID, EXPECTED_SETREGID);
    assert_eq!(SetResUid::ID, EXPECTED_SETRESUID);
    assert_eq!(SetResGid::ID, EXPECTED_SETRESGID);
}

#[test]
fn negative_one_id_arguments_leave_credentials_unchanged() {
    assert_eq!(optional_id(u64::MAX), None);
    assert_eq!(optional_id(u64::from(u32::MAX)), None);
    assert_eq!(optional_id(1234), Some(1234));
}