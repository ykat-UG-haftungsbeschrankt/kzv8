/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{combine_usage, encode_usage, Getrusage, RUSAGE_BOTH, RUSAGE_CHILDREN, RUSAGE_SELF, RUSAGE_THREAD, RUSAGE_LENGTH};
use crate::kernel::process::ResourceUsageSnapshot;

#[cfg(target_arch = "x86_64")]
const EXPECTED_GETRUSAGE: u64 = 98;
#[cfg(target_arch = "aarch64")]
const EXPECTED_GETRUSAGE: u64 = 165;

#[test]
fn syscall_id_matches_linux_architecture_abi() {
    assert_eq!(Getrusage::ID, EXPECTED_GETRUSAGE);
}

#[test]
fn usage_matches_linux_lp64_layout_and_units() {
    let encoded = encode_usage(ResourceUsageSnapshot {
        user_cpu_time_ns: 2_500_000_123,
        system_cpu_time_ns: 3_750_000_999,
        max_rss_bytes: 4097,
        minor_page_faults: 4,
        major_page_faults: 5,
        swaps: 6,
        input_blocks: 7,
        output_blocks: 8,
        messages_sent: 9,
        messages_received: 10,
        signals_received: 11,
        voluntary_context_switches: 12,
        involuntary_context_switches: 13,
    })
    .unwrap();

    assert_eq!(encoded.len(), 144);
    let field = |index: usize| {
        let start = index * 8;
        i64::from_ne_bytes(encoded[start..start + 8].try_into().unwrap())
    };
    assert_eq!(field(0), 2);
    assert_eq!(field(1), 500_000);
    assert_eq!(field(2), 3);
    assert_eq!(field(3), 750_000);
    assert_eq!(field(4), 4);
    assert_eq!(field(8), 4);
    assert_eq!(field(17), 13);
    assert_eq!(RUSAGE_LENGTH, 144);
}

#[test]
fn rusage_selectors_match_linux_values() {
    assert_eq!(RUSAGE_SELF, 0);
    assert_eq!(RUSAGE_CHILDREN, -1);
    assert_eq!(RUSAGE_THREAD, 1);
    assert_eq!(RUSAGE_BOTH, -2);
}

#[test]
fn rusage_both_combines_cpu_and_counters_and_keeps_maximum_rss() {
    let combined = combine_usage(
        ResourceUsageSnapshot {
            user_cpu_time_ns: 1,
            max_rss_bytes: 4096,
            minor_page_faults: 2,
            ..ResourceUsageSnapshot::default()
        },
        ResourceUsageSnapshot {
            system_cpu_time_ns: 3,
            max_rss_bytes: 2048,
            major_page_faults: 4,
            ..ResourceUsageSnapshot::default()
        },
    );
    assert_eq!(combined.user_cpu_time_ns, 1);
    assert_eq!(combined.system_cpu_time_ns, 3);
    assert_eq!(combined.max_rss_bytes, 4096);
    assert_eq!(combined.minor_page_faults, 2);
    assert_eq!(combined.major_page_faults, 4);
}