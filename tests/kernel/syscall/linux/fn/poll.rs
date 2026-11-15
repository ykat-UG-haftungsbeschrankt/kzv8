/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{poll_deadline, ready_events, Poll};

#[test]
fn syscall_id_matches_linux_x86_64_abi() {
    assert_eq!(Poll::ID, 7);
}

#[test]
fn reports_console_readiness_and_invalid_descriptors() {
    assert_eq!(ready_events(0, 0x0001, false, false, false), 0);
    assert_eq!(ready_events(0, 0x0001, true, false, false), 0x0001);
    assert_eq!(ready_events(1, 0x0004, false, true, false), 0x0004);
    assert_eq!(ready_events(3, 0x0001, false, false, true), 0);
    assert_eq!(ready_events(3, 0x0001, false, false, false), 0x0020);
}

#[test]
fn ignores_negative_poll_descriptors() {
    assert_eq!(ready_events(-1, 0x0001, true, true, false), 0);
}

#[test]
fn poll_timeout_deadline_matches_linux_timeout_values() {
    assert_eq!(poll_deadline(-1, 100), None);
    assert_eq!(poll_deadline(0, 100), Some(100));
    assert_eq!(poll_deadline(25, 100), Some(25_000_100));
}