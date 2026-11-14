/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use core::borrow::Borrow;
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

pub(crate) struct HashMap<K, V, const INITIAL_CAPACITY: usize = 8> {
	address: usize,
	allocation_length: usize,
	capacity: usize,
	length: usize,
	static_buckets: [Bucket<K, V>; INITIAL_CAPACITY],
	is_static: bool,
	marker: PhantomData<(K, V)>,
}

impl<K, V, const INITIAL_CAPACITY: usize> HashMap<K, V, INITIAL_CAPACITY> {
	pub(crate) const fn new() -> Self {
		assert!(INITIAL_CAPACITY > 0 && INITIAL_CAPACITY.is_power_of_two());
		Self {
			address: 0,
			allocation_length: 0,
			capacity: INITIAL_CAPACITY,
			length: 0,
			static_buckets: [const { LinkedList::new() }; INITIAL_CAPACITY],
			is_static: true,
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

	pub(crate) fn for_each_mut(&mut self, mut visit: impl FnMut(&K, &mut V)) {
		for index in 0..self.capacity {
			for entry in self.bucket_mut(index).iter_mut() {
				visit(&entry.key, &mut entry.value);
			}
		}
	}

	pub(crate) fn try_for_each<E>(
		&self,
		mut visit: impl FnMut(&K, &V) -> Result<(), E>,
	) -> Result<(), E> {
		for index in 0..self.capacity {
			for entry in self.bucket(index).iter() {
				visit(&entry.key, &entry.value)?;
			}
		}
		Ok(())
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
		if self.is_static {
			return &self.static_buckets[index];
		}
		// SAFETY: a nonzero capacity means the reserved table is initialized and index is bounded.
		unsafe { &*((self.address as *const Bucket<K, V>).add(index)) }
	}

	fn bucket_mut(&mut self, index: usize) -> &mut Bucket<K, V> {
		debug_assert!(index < self.capacity);
		if self.is_static {
			return &mut self.static_buckets[index];
		}
		// SAFETY: a nonzero capacity means the reserved table is initialized and index is bounded.
		unsafe { &mut *((self.address as *mut Bucket<K, V>).add(index)) }
	}

	fn release_table(address: usize, capacity: usize, allocation_length: usize) {
		let buckets = address as *mut Bucket<K, V>;
		for index in 0..capacity {
			// SAFETY: every bucket was initialized by allocate_buckets and is dropped exactly once.
			unsafe { ptr::drop_in_place(buckets.add(index)) };
		}
		let _released = crate::kernel::memory::r#virtual::release(address, allocation_length);
	}

	fn release_buckets(&mut self) {
		if self.is_static {
			return;
		}
		Self::release_table(self.address, self.capacity, self.allocation_length);
	}
}

impl<K: Eq + Hash, V, const INITIAL_CAPACITY: usize> HashMap<K, V, INITIAL_CAPACITY> {
	fn bucket_index<Q: ?Sized + Hash>(key: &Q, capacity: usize) -> usize {
		let mut hasher = FnvHasher::new();
		key.hash(&mut hasher);
		(hasher.finish() as usize) & (capacity - 1)
	}

	fn resize(&mut self, new_capacity: usize) -> bool {
		let new_capacity = new_capacity.max(INITIAL_CAPACITY);
		if new_capacity == self.capacity {
			return true;
		}
		if !new_capacity.is_power_of_two() {
			return false;
		}
		if new_capacity == INITIAL_CAPACITY {
			if self.is_static {
				return true;
			}
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
						Self::bucket_index(&entry.key, INITIAL_CAPACITY)
					};
					self.static_buckets[new_index].push_back(node);
				}
			}
			Self::release_table(old_address, old_capacity, old_allocation_length);
			self.address = 0;
			self.allocation_length = 0;
			self.capacity = INITIAL_CAPACITY;
			self.is_static = true;
			return true;
		}
		let Some((new_address, new_allocation_length)) = Self::allocate_buckets(new_capacity) else {
			return false;
		};
		let new_buckets = new_address as *mut Bucket<K, V>;
		let old_is_static = self.is_static;
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

		if !old_is_static {
			Self::release_table(old_address, old_capacity, old_allocation_length);
		}

		self.address = new_address;
		self.allocation_length = new_allocation_length;
		self.capacity = new_capacity;
		self.is_static = false;
		true
	}

	fn prepare_insert(&mut self) -> bool {
		let load_limit = self.capacity - self.capacity / 4;
		if self.length < load_limit {
			return true;
		}
		let Some(new_capacity) = self.capacity.checked_mul(2) else {
			return false;
		};
		self.resize(new_capacity)
	}

	fn maybe_shrink(&mut self) {
		if self.is_static {
			return;
		}
		let mut new_capacity = self.capacity;
		while new_capacity > INITIAL_CAPACITY && self.length < new_capacity / 4 {
			new_capacity /= 2;
		}
		if new_capacity < self.capacity {
			let _resized = self.resize(new_capacity);
		}
	}

	pub(crate) fn insert(&mut self, key: K, value: V) -> Result<Option<V>, (K, V)> {
		let index = Self::bucket_index(&key, self.capacity);
		for entry in self.bucket_mut(index).iter_mut() {
			if entry.key == key {
				return Ok(Some(core::mem::replace(&mut entry.value, value)));
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

	pub(crate) fn get<Q: ?Sized>(&self, key: &Q) -> Option<&V>
	where
		K: Borrow<Q>,
		Q: Eq + Hash,
	{
		if self.capacity == 0 {
			return None;
		}
		let index = Self::bucket_index(key, self.capacity);
		for entry in self.bucket(index).iter() {
			if <K as Borrow<Q>>::borrow(&entry.key) == key {
				return Some(&entry.value);
			}
		}
		None
	}

	pub(crate) fn get_mut<Q: ?Sized>(&mut self, key: &Q) -> Option<&mut V>
	where
		K: Borrow<Q>,
		Q: Eq + Hash,
	{
		if self.capacity == 0 {
			return None;
		}
		let index = Self::bucket_index(key, self.capacity);
		for entry in self.bucket_mut(index).iter_mut() {
			if <K as Borrow<Q>>::borrow(&entry.key) == key {
				return Some(&mut entry.value);
			}
		}
		None
	}

	pub(crate) fn contains_key<Q: ?Sized>(&self, key: &Q) -> bool
	where
		K: Borrow<Q>,
		Q: Eq + Hash,
	{
		self.get(key).is_some()
	}

	pub(crate) fn remove<Q: ?Sized>(&mut self, key: &Q) -> Option<V>
	where
		K: Borrow<Q>,
		Q: Eq + Hash,
	{
		if self.capacity == 0 {
			return None;
		}
		let index = Self::bucket_index(key, self.capacity);
		let node = self
			.bucket(index)
			.find(|entry| <K as Borrow<Q>>::borrow(&entry.key) == key)?;
		// SAFETY: node was found in this bucket and has not been unlinked.
		let entry = unsafe { ListNode::remove(self.bucket_mut(index), node) }?.into_value();
		self.length -= 1;
		self.maybe_shrink();
		Some(entry.value)
	}
}

impl<K, V, const INITIAL_CAPACITY: usize> Drop for HashMap<K, V, INITIAL_CAPACITY> {
	fn drop(&mut self) {
		self.release_buckets();
	}
}

#[cfg(test)]
#[path = "../../tests/kernel/lib/hashmap.rs"]
mod tests;
