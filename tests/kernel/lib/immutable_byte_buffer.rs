/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::ImmutableByteBuffer;

#[test]
fn immutable_byte_buffer_copies_input_and_handles_empty_slices() {
	let input = [0, 0x80, 0xff];
	let buffer = ImmutableByteBuffer::try_from_slice(&input).unwrap();
	assert_eq!(buffer.as_bytes(), input.as_slice());

	let empty = ImmutableByteBuffer::try_from_slice(&[]).unwrap();
	assert!(empty.as_bytes().is_empty());
}