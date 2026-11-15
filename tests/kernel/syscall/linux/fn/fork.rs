/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{validate_clone_arguments, CloneProcess, ForkProcess, VforkProcess};
use crate::kernel::error::Error;
use crate::kernel::signal::SIGCHLD;

#[test]
fn syscall_ids_match_linux_x86_64_abi() {
    assert_eq!(ForkProcess::ID, 57);
    assert_eq!(CloneProcess::ID, 56);
    assert_eq!(VforkProcess::ID, 58);
}

#[test]
fn accepts_only_fork_style_clone_arguments() {
    let mut arguments = [0; 6];
    arguments[0] = u64::from(SIGCHLD);
    arguments[5] = 0xfeed;
    assert_eq!(validate_clone_arguments(arguments), Ok(()));

    arguments[0] |= 0x100;
    assert_eq!(validate_clone_arguments(arguments), Err(Error::Inval));
    arguments[0] = u64::from(SIGCHLD);
    arguments[1] = 0x1000;
    assert_eq!(validate_clone_arguments(arguments), Err(Error::Inval));
}