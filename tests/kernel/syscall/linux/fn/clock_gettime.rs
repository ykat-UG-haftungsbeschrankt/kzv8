/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{ClockGetTime, CLOCK_MONOTONIC, CLOCK_MONOTONIC_RAW, CLOCK_PROCESS_CPUTIME_ID, CLOCK_REALTIME, CLOCK_THREAD_CPUTIME_ID};

#[cfg(target_arch = "x86_64")]
const EXPECTED_CLOCK_GETTIME: u64 = 228;
#[cfg(target_arch = "aarch64")]
const EXPECTED_CLOCK_GETTIME: u64 = 113;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(ClockGetTime::ID, EXPECTED_CLOCK_GETTIME);
}

#[test]
fn supported_clock_ids_include_process_and_thread_cpu_time() {
    assert_eq!(CLOCK_REALTIME, 0);
    assert_eq!(CLOCK_MONOTONIC, 1);
    assert_eq!(CLOCK_PROCESS_CPUTIME_ID, 2);
    assert_eq!(CLOCK_THREAD_CPUTIME_ID, 3);
    assert_eq!(CLOCK_MONOTONIC_RAW, 4);
}