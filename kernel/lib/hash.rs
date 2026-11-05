/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

/*
 * FnvHasher folds input bytes into a 64-bit hash state:
 *
 * state -- xor byte --> multiply --> state
 */

use core::hash::Hasher;

pub(crate) struct FnvHasher(u64);

impl FnvHasher {
	pub(crate) const fn new() -> Self {
		Self(0xcbf2_9ce4_8422_2325)
	}
}

impl Hasher for FnvHasher {
	fn finish(&self) -> u64 {
		self.0
	}

	fn write(&mut self, bytes: &[u8]) {
		for byte in bytes {
			self.0 ^= u64::from(*byte);
			self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
		}
	}

	fn write_usize(&mut self, value: usize) {
		let mut value = self.0 ^ value as u64;
		value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
		value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
		self.0 = value ^ (value >> 31);
	}
}
