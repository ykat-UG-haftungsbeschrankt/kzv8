/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{parse_options, wait_status, Wait4};
use crate::kernel::error::Error;
use crate::kernel::signal::{CLD_CONTINUED, CLD_EXITED, CLD_KILLED, CLD_STOPPED};

#[cfg(target_arch = "x86_64")]
const EXPECTED_WAIT4: u64 = 61;
#[cfg(target_arch = "aarch64")]
const EXPECTED_WAIT4: u64 = 260;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(Wait4::ID, EXPECTED_WAIT4);
}

#[test]
fn accepts_supported_wait_options_and_rejects_unknown_bits() {
    let options = parse_options(0x0b).unwrap();
    assert!(options.no_hang);
    assert!(options.stopped);
    assert!(options.continued);
    assert_eq!(parse_options(0x04).unwrap_err(), Error::Inval);
}

#[test]
fn encodes_wait_status_for_exit_signal_stop_and_continue() {
    assert_eq!(wait_status(CLD_EXITED, 7), 7 << 8);
    assert_eq!(wait_status(CLD_KILLED, 9), 9);
    assert_eq!(wait_status(CLD_STOPPED, 19), (19 << 8) | 0x7f);
    assert_eq!(wait_status(CLD_CONTINUED, 18), 0xffff);
}