/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{decode_action, encode_action, RtSigaction, SA_RESTORER, SA_RESTART, SA_SIGINFO};
use crate::kernel::error::Error;
use crate::kernel::signal::{SignalAction, SignalSet};

#[cfg(target_arch = "x86_64")]
const EXPECTED_RT_SIGACTION: u64 = 13;
#[cfg(target_arch = "aarch64")]
const EXPECTED_RT_SIGACTION: u64 = 134;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(RtSigaction::ID, EXPECTED_RT_SIGACTION);
}

#[test]
fn action_round_trips_supported_handler_mask_and_flags() {
    let action = SignalAction {
        handler: 0x1234,
        mask: SignalSet::from_signal(2).insert(64),
        flags: SA_RESTART as u32 | SA_SIGINFO as u32,
    };
    assert_eq!(decode_action(&encode_action(action)), Ok(action));
}

#[test]
fn unsupported_action_flags_return_enosys() {
    let mut bytes = [0; 32];
    bytes[8..16].copy_from_slice(&SA_RESTORER.to_ne_bytes());
    assert_eq!(decode_action(&bytes), Err(Error::NoSys));
}