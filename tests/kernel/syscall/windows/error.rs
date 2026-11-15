/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::ntstatus;
use crate::kernel::error::Error;

#[test]
fn maps_errors_to_ntstatus_values() {
    assert_eq!(ntstatus(Error::NoSys), 0xc000_0002);
    assert_eq!(ntstatus(Error::Perm), 0xc000_0022);
    assert_eq!(ntstatus(Error::NoEnt), 0xc000_0034);
    assert_eq!(ntstatus(Error::Fault), 0xc000_0005);
    assert_eq!(ntstatus(Error::Busy), 0x8000_0011);
    assert_eq!(ntstatus(Error::Success), 0);
}