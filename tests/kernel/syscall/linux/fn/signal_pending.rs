/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{pending_blocked_mask, RtSigpending};
use crate::kernel::signal::SignalSet;

#[cfg(target_arch = "x86_64")]
const EXPECTED_RT_SIGPENDING: u64 = 127;
#[cfg(target_arch = "aarch64")]
const EXPECTED_RT_SIGPENDING: u64 = 136;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(RtSigpending::ID, EXPECTED_RT_SIGPENDING);
}

#[test]
fn rt_sigpending_reports_only_blocked_pending_signals() {
    let thread_pending = SignalSet::from_signal(2).insert(3);
    let process_pending = SignalSet::from_signal(4);
    let blocked = SignalSet::from_signal(3).insert(4);
    assert_eq!(pending_blocked_mask(thread_pending, process_pending, blocked), blocked.bits());
}