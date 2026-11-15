/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{string_slices, Execve};

#[test]
fn syscall_number_matches_x86_64_linux_abi() {
    assert_eq!(Execve::ID, 59);
}

#[test]
fn string_slices_borrow_copied_exec_strings() {
    let strings = alloc::vec![b"/fs/mc".to_vec(), b"-V".to_vec()];
    let expected: [&[u8]; 2] = [b"/fs/mc".as_slice(), b"-V".as_slice()];
    assert_eq!(string_slices(&strings).unwrap().as_slice(), expected.as_slice());
}