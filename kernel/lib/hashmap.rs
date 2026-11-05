/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::mem::{align_of, size_of};
use core::ptr;

use crate::kernel::lib::hash::FnvHasher;
use crate::kernel::lib::list::{LinkedList, ListNode};

struct Entry<K, V> {
	key: K,
	value: V,
}

type Bucket<K, V> = LinkedList<Entry<K, V>>;

pub(crate) struct HashMap<K, V> {
	address: usize,
	allocation_length: usize,
	capacity: usize,
	length: usize,
	marker: PhantomData<(K, V)>,
}

impl<K, V> HashMap<K, V> {
	const INITIAL_CAPACITY: usize = 8;

	pub(crate) const fn new() -> Self {
		Self {
			address: 0,
			allocation_length: 0,
			capacity: 0,
			length: 0,
			marker: PhantomData,
		}
	}

	pub(crate) fn len(&self) -> usize {
		self.length
	}

	pub(crate) fn is_empty(&self) -> bool {
		self.length == 0
	}

	pub(crate) fn capacity(&self) -> usize {
		self.capacity
	}

	pub(crate) fn clear(&mut self) {
		for index in 0..self.capacity {
			self.bucket_mut(index).clear();
		}
		self.length = 0;
	}

	fn allocate_buckets(capacity: usize) -> Option<(usize, usize)> {
		let allocation_length = capacity.checked_mul(size_of::<Bucket<K, V>>())?;
		let address = crate::kernel::memory::r#virtual::reserve(
			allocation_length,
			align_of::<Bucket<K, V>>(),
			2,
		)?;
		let buckets = address as *mut Bucket<K, V>;
		for index in 0..capacity {
			// SAFETY: the reservation covers every bucket slot, and each slot is initialized once.
			unsafe { buckets.add(index).write(LinkedList::new()) };
		}
		Some((address, allocation_length))
	}

	fn bucket(&self, index: usize) -> &Bucket<K, V> {
		debug_assert!(index < self.capacity);
		// SAFETY: a nonzero capacity means the reserved table is initialized and index is bounded.
		unsafe { &*((self.address as *const Bucket<K, V>).add(index)) }
	}

	fn bucket_mut(&mut self, index: usize) -> &mut Bucket<K, V> {
		debug_assert!(index < self.capacity);
		// SAFETY: a nonzero capacity means the reserved table is initialized and index is bounded.
		unsafe { &mut *((self.address as *mut Bucket<K, V>).add(index)) }
	}

	fn release_buckets(&mut self) {
		if self.address == 0 {
			return;
		}

		let buckets = self.address as *mut Bucket<K, V>;
		for index in 0..self.capacity {
			// SAFETY: each slot was initialized by allocate_buckets and is dropped exactly once.
			unsafe { ptr::drop_in_place(buckets.add(index)) };
		}
		let address = self.address;
		let allocation_length = self.allocation_length;
		self.address = 0;
		self.allocation_length = 0;
		self.capacity = 0;
		self.length = 0;
		let _released = crate::kernel::memory::r#virtual::release(address, allocation_length);
	}
}

impl<K: Eq + Hash, V> HashMap<K, V> {
	fn bucket_index(key: &K, capacity: usize) -> usize {
		let mut hasher = FnvHasher::new();
		key.hash(&mut hasher);
		(hasher.finish() as usize) & (capacity - 1)
	}

	fn resize(&mut self, new_capacity: usize) -> bool {
		let Some((new_address, new_allocation_length)) = Self::allocate_buckets(new_capacity) else {
			return false;
		};
		let new_buckets = new_address as *mut Bucket<K, V>;
		let old_address = self.address;
		let old_capacity = self.capacity;
		let old_allocation_length = self.allocation_length;

		for old_index in 0..old_capacity {
			loop {
				let Some(node) = self.bucket_mut(old_index).pop_front() else {
					break;
				};
				let new_index = {
					let entry: &Entry<K, V> = &**node;
					Self::bucket_index(&entry.key, new_capacity)
				};
				// SAFETY: new_buckets contains initialized slots and the node is detached from its old list.
				unsafe { (*new_buckets.add(new_index)).push_back(node) };
			}
		}

		if old_address != 0 {
			let old_buckets = old_address as *mut Bucket<K, V>;
			for index in 0..old_capacity {
				// SAFETY: all entries were moved out, and each old bucket is dropped exactly once.
				unsafe { ptr::drop_in_place(old_buckets.add(index)) };
			}
			let _released =
				crate::kernel::memory::r#virtual::release(old_address, old_allocation_length);
		}

		self.address = new_address;
		self.allocation_length = new_allocation_length;
		self.capacity = new_capacity;
		true
	}

	fn prepare_insert(&mut self) -> bool {
		if self.capacity == 0 {
			return self.resize(Self::INITIAL_CAPACITY);
		}
		let load_limit = self.capacity - self.capacity / 4;
		if self.length < load_limit {
			return true;
		}
		let Some(new_capacity) = self.capacity.checked_mul(2) else {
			return false;
		};
		self.resize(new_capacity)
	}

	pub(crate) fn insert(&mut self, key: K, value: V) -> Result<Option<V>, (K, V)> {
		if self.capacity != 0 {
			let index = Self::bucket_index(&key, self.capacity);
			for entry in self.bucket_mut(index).iter_mut() {
				if entry.key == key {
					return Ok(Some(core::mem::replace(&mut entry.value, value)));
				}
			}
		}

		if !self.prepare_insert() {
			return Err((key, value));
		}
		let index = Self::bucket_index(&key, self.capacity);
		self.bucket_mut(index)
			.push_back(Box::new(ListNode::new(Entry { key, value })));
		self.length += 1;
		Ok(None)
	}

	pub(crate) fn get(&self, key: &K) -> Option<&V> {
		if self.capacity == 0 {
			return None;
		}
		let index = Self::bucket_index(key, self.capacity);
		for entry in self.bucket(index).iter() {
			if entry.key == *key {
				return Some(&entry.value);
			}
		}
		None
	}

	pub(crate) fn get_mut(&mut self, key: &K) -> Option<&mut V> {
		if self.capacity == 0 {
			return None;
		}
		let index = Self::bucket_index(key, self.capacity);
		for entry in self.bucket_mut(index).iter_mut() {
			if entry.key == *key {
				return Some(&mut entry.value);
			}
		}
		None
	}

	pub(crate) fn contains_key(&self, key: &K) -> bool {
		self.get(key).is_some()
	}

	pub(crate) fn remove(&mut self, key: &K) -> Option<V> {
		if self.capacity == 0 {
			return None;
		}
		let index = Self::bucket_index(key, self.capacity);
		let node = self.bucket(index).find(|entry| entry.key == *key)?;
		// SAFETY: node was found in this bucket and has not been unlinked.
		let entry = unsafe { ListNode::remove(self.bucket_mut(index), node) }?.into_value();
		self.length -= 1;
		Some(entry.value)
	}
}

impl<K, V> Drop for HashMap<K, V> {
	fn drop(&mut self) {
		self.release_buckets();
	}
}

