/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{update_mask, RtSigprocmask, SIG_BLOCK, SIG_SETMASK, SIG_UNBLOCK};
use crate::kernel::error::Error;
use crate::kernel::signal::{SignalSet, SIGKILL, SIGSTOP};

#[cfg(target_arch = "x86_64")]
const EXPECTED_RT_SIGPROCMASK: u64 = 14;
#[cfg(target_arch = "aarch64")]
const EXPECTED_RT_SIGPROCMASK: u64 = 135;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(RtSigprocmask::ID, EXPECTED_RT_SIGPROCMASK);
}

#[test]
fn masks_support_block_unblock_and_replace_without_uncatchable_signals() {
    let initial = SignalSet::from_signal(2);
    let requested = SignalSet::from_signal(3)
        .insert(SIGKILL)
        .insert(SIGSTOP);
    assert_eq!(
        update_mask(initial, SIG_BLOCK, requested),
        Ok(initial.union(SignalSet::from_signal(3)))
    );
    assert_eq!(
        update_mask(initial.union(SignalSet::from_signal(3)), SIG_UNBLOCK, SignalSet::from_signal(2)),
        Ok(SignalSet::from_signal(3))
    );
    assert_eq!(
        update_mask(initial, SIG_SETMASK, requested),
        Ok(SignalSet::from_signal(3))
    );
    assert_eq!(update_mask(initial, 3, requested), Err(Error::Inval));
}