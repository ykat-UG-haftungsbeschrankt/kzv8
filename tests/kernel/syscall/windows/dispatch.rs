/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{dispatch_call, SyscallFrame};
use crate::kernel::error::Error;

#[test]
fn service_ids_match_windows_11_24h2_x64() {
    assert_eq!(super::r#fn::delay_execution::DelayExecution::ID, 0x0034);
    assert_eq!(
        super::r#fn::query_performance_counter::QueryPerformanceCounter::ID,
        0x0031
    );
    assert_eq!(super::r#fn::terminate_process::TerminateProcess::ID, 0x002c);
    assert_eq!(super::r#fn::yield_execution::YieldExecution::ID, 0x0046);
}

#[test]
fn unknown_service_is_not_implemented() {
    let error = dispatch_call(SyscallFrame {
        number: u64::MAX,
        arguments: [0; 6],
        windows_arguments: [0; 4],
        result: 0,
        errno: 0,
    })
    .unwrap_err();

    assert_eq!(error, Error::NoSys);
    assert_eq!(super::error::ntstatus(error), 0xc000_0002);
}