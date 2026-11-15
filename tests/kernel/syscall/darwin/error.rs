/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::errno;
use crate::kernel::error::Error;

#[test]
fn maps_errors_to_darwin_errno_values() {
    assert_eq!(errno(Error::Perm), 1);
    assert_eq!(errno(Error::NoEnt), 2);
    assert_eq!(errno(Error::Deadlk), 11);
    assert_eq!(errno(Error::Again), 35);
    assert_eq!(errno(Error::WouldBlock), 35);
    assert_eq!(errno(Error::NoSys), 78);
}

#[test]
fn maps_non_darwin_errors_to_invalid_argument() {
    assert_eq!(errno(Error::Min), 22);
    assert_eq!(errno(Error::Max), 22);
}