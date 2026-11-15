/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{pid_argument, GetPgid, GetSid, SetPgid, SetSid};

#[cfg(target_arch = "x86_64")]
const EXPECTED_GETPGID: u64 = 121;
#[cfg(target_arch = "x86_64")]
const EXPECTED_GETSID: u64 = 124;
#[cfg(target_arch = "x86_64")]
const EXPECTED_SETPGID: u64 = 109;
#[cfg(target_arch = "x86_64")]
const EXPECTED_SETSID: u64 = 112;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETPGID: u64 = 155;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETSID: u64 = 156;
#[cfg(target_arch = "aarch64")]
const EXPECTED_SETPGID: u64 = 154;
#[cfg(target_arch = "aarch64")]
const EXPECTED_SETSID: u64 = 157;

#[test]
fn syscall_ids_match_linux_architecture_abi() {
    assert_eq!(GetPgid::ID, EXPECTED_GETPGID);
    assert_eq!(GetSid::ID, EXPECTED_GETSID);
    assert_eq!(SetPgid::ID, EXPECTED_SETPGID);
    assert_eq!(SetSid::ID, EXPECTED_SETSID);
}

#[test]
fn pid_arguments_use_linux_pid_t_width_and_sign() {
    assert_eq!(pid_argument(0), 0);
    assert_eq!(pid_argument(42), 42);
    assert_eq!(pid_argument(u64::MAX), -1);
}