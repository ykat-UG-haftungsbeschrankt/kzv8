/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use crate::kernel::error::Error;

pub(crate) struct ImmutableByteBuffer {
	bytes: Vec<u8>,
}

impl ImmutableByteBuffer {
	pub(crate) fn try_from_slice(bytes: &[u8]) -> Result<Self, Error> {
		let mut owned_bytes = Vec::new();
		owned_bytes
			.try_reserve_exact(bytes.len())
			.map_err(|_| Error::NoMem)?;
		owned_bytes.extend_from_slice(bytes);
		Ok(Self { bytes: owned_bytes })
	}

	pub(crate) fn as_bytes(&self) -> &[u8] {
		&self.bytes
	}
}

#[cfg(test)]
#[path = "../../tests/kernel/lib/immutable_byte_buffer.rs"]
mod tests;