/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{STACK_ARGUMENT_COUNT, STACK_ARGUMENT_OFFSET, WriteFile};

#[test]
fn syscall_id_matches_windows_x64_ntwritefile() {
	assert_eq!(WriteFile::ID, 0x0008);
}

#[test]
fn stack_arguments_follow_windows_x64_shadow_space() {
	assert_eq!(STACK_ARGUMENT_OFFSET, 40);
	assert_eq!(STACK_ARGUMENT_COUNT, 5);
}