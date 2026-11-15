/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::resolve_at_current_directory;
use crate::kernel::error::Error;

#[test]
fn relative_path_is_joined_to_process_current_directory() {
    assert_eq!(
        resolve_at_current_directory("../file".into(), b"/home/user"),
        Ok("/home/user/../file".into())
    );
}

#[test]
fn absolute_path_is_not_prefixed_with_current_directory() {
    assert_eq!(
        resolve_at_current_directory("/tmp/file".into(), b"/home/user"),
        Ok("/tmp/file".into())
    );
}

#[test]
fn empty_path_and_oversized_join_are_rejected() {
    assert_eq!(
        resolve_at_current_directory(String::new(), b"/"),
        Err(Error::NoEnt)
    );
    let path = alloc::format!("/{}", "x".repeat(4096));
    assert_eq!(
        resolve_at_current_directory(path, b"/home/user"),
        Err(Error::NameTooLong)
    );
}