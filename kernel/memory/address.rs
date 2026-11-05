/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

const NO_INDEX: usize = usize::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UserAddress {
	index: u32,
	generation: u32,
}

impl UserAddress {
	pub(crate) const fn raw(self) -> u64 {
		(u64::from(self.generation) << 32) | u64::from(self.index)
	}

	pub(crate) const fn from_raw(raw: u64) -> Self {
		Self {
			index: raw as u32,
			generation: (raw >> 32) as u32,
		}
	}
}

struct Entry<T> {
	value: Option<T>,
	generation: u32,
	next_free: usize,
}

impl<T> Entry<T> {
	const fn empty() -> Self {
		Self {
			value: None,
			generation: 0,
			next_free: NO_INDEX,
		}
	}
}

pub(crate) struct AddressIndex<T, const N: usize> {
	entries: [Entry<T>; N],
	free_head: usize,
	length: usize,
}

impl<T, const N: usize> AddressIndex<T, N> {
	pub(crate) const fn new() -> Self {
		assert!(N > 0 && N <= u32::MAX as usize);
		let mut entries = [const { Entry::empty() }; N];
		let mut index = 0;
		while index < N {
			entries[index].next_free = if index + 1 < N {
				index + 1
			} else {
				NO_INDEX
			};
			index += 1;
		}
		Self {
			entries,
			free_head: 0,
			length: 0,
		}
	}

	pub(crate) const fn len(&self) -> usize {
		self.length
	}

	pub(crate) const fn capacity(&self) -> usize {
		N
	}

	pub(crate) fn insert(&mut self, value: T) -> Result<UserAddress, T> {
		let index = self.free_head;
		if index == NO_INDEX {
			return Err(value);
		}
		let next_free = self.entries[index].next_free;
		let generation = self.entries[index].generation.wrapping_add(1).max(1);
		self.entries[index] = Entry {
			value: Some(value),
			generation,
			next_free: NO_INDEX,
		};
		self.free_head = next_free;
		self.length += 1;
		Ok(UserAddress {
			index: index as u32,
			generation,
		})
	}

	pub(crate) fn get(&self, address: UserAddress) -> Option<&T> {
		let entry = self.entries.get(address.index as usize)?;
		if entry.generation != address.generation {
			return None;
		}
		entry.value.as_ref()
	}

	pub(crate) fn get_mut(&mut self, address: UserAddress) -> Option<&mut T> {
		let entry = self.entries.get_mut(address.index as usize)?;
		if entry.generation != address.generation {
			return None;
		}
		entry.value.as_mut()
	}

	pub(crate) fn remove(&mut self, address: UserAddress) -> Option<T> {
		let entry = self.entries.get_mut(address.index as usize)?;
		if entry.generation != address.generation {
			return None;
		}
		let value = entry.value.take()?;
		entry.next_free = self.free_head;
		self.free_head = address.index as usize;
		self.length -= 1;
		Some(value)
	}
}

#[cfg(test)]
mod tests {
	use super::{AddressIndex, UserAddress};

	#[test]
	fn resolves_and_rejects_stale_handles_after_reuse() {
		let mut index = AddressIndex::<usize, 1>::new();
		let first = index.insert(0x1000).unwrap();
		assert_eq!(index.get(first), Some(&0x1000));
		assert_eq!(index.get(UserAddress::from_raw(first.raw())), Some(&0x1000));
		assert_eq!(index.remove(first), Some(0x1000));
		assert_eq!(index.get(first), None);

		let second = index.insert(0x2000).unwrap();
		assert_ne!(first, second);
		assert_eq!(index.get(first), None);
		assert_eq!(index.get(second), Some(&0x2000));
	}

	#[test]
	fn rejects_invalid_handles_and_preserves_values_when_full() {
		let mut index = AddressIndex::<usize, 1>::new();
		assert_eq!(index.get(UserAddress::from_raw(0)), None);
		let _address = index.insert(0x1000).unwrap();
		assert_eq!(index.insert(0x2000), Err(0x2000));
		assert_eq!(index.len(), 1);
		assert_eq!(index.capacity(), 1);
	}
}
