/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::marker::PhantomData;
use core::mem::{size_of, MaybeUninit};

const PAGE_SIZE: usize = crate::kernel::memory::r#virtual::PAGE_SIZE;

#[repr(C)]
struct Page {
	next: *mut Page,
	start: usize,
	end: usize,
}

impl Page {
	const EMPTY: Self = Self {
		next: core::ptr::null_mut(),
		start: 0,
		end: 0,
	};
}

pub(crate) struct Queue {
	head: *mut Page,
	tail: *mut Page,
	length: usize,
}

// SAFETY: Queue exclusively owns its pages, and moving it transfers that ownership.
unsafe impl Send for Queue {}

impl Queue {
	pub(crate) const fn new() -> Self {
		Self {
			head: core::ptr::null_mut(),
			tail: core::ptr::null_mut(),
			length: 0,
		}
	}

	pub(crate) const fn len(&self) -> usize {
		self.length
	}

	pub(crate) const fn is_empty(&self) -> bool {
		self.length == 0
	}

	fn page_capacity() -> Option<usize> {
		PAGE_SIZE
			.checked_sub(size_of::<Page>())
			.filter(|capacity| *capacity != 0)
	}

	fn data_ptr(page: *mut Page) -> *mut u8 {
		page.cast::<u8>().wrapping_add(size_of::<Page>())
	}

	fn allocate_page() -> Option<*mut Page> {
		Self::page_capacity()?;
		let address = crate::kernel::memory::r#virtual::reserve(PAGE_SIZE, PAGE_SIZE, 2)?;
		let page = address as *mut Page;
		// SAFETY: reserve returns an aligned writable page that is exclusively owned by this queue.
		unsafe { page.write(Page::EMPTY) };
		Some(page)
	}

	fn ensure_tail_room(&mut self, page_capacity: usize) -> bool {
		if !self.tail.is_null() {
			let tail = self.tail;
			// SAFETY: tail points to a live page owned by this queue.
			if unsafe { (*tail).end < page_capacity } {
				return true;
			}
		}

		let Some(new_page) = Self::allocate_page() else {
			return false;
		};
		if self.tail.is_null() {
			self.head = new_page;
		} else {
			let tail = self.tail;
			// SAFETY: tail belongs to this queue and new_page is a live allocated page.
			unsafe { (*tail).next = new_page };
		}
		self.tail = new_page;
		true
	}

	pub(crate) fn write(&mut self, bytes: &[u8]) -> usize {
		let Some(page_capacity) = Self::page_capacity() else {
			return 0;
		};
		let mut written = 0;
		while written < bytes.len() && self.length < usize::MAX {
			if !self.ensure_tail_room(page_capacity) {
				break;
			}

			let tail = self.tail;
			// SAFETY: tail points to a live page owned by this queue.
			let end = unsafe { (*tail).end };
			let count = (bytes.len() - written)
				.min(page_capacity - end)
				.min(usize::MAX - self.length);
			debug_assert_ne!(count, 0);
			// SAFETY: the input slice and page range are valid and do not overlap.
			unsafe {
				core::ptr::copy_nonoverlapping(
					bytes.as_ptr().add(written),
					Self::data_ptr(tail).add(end),
					count,
				);
				(*tail).end = end + count;
			}
			self.length += count;
			written += count;
		}
		written
	}

	pub(crate) fn write_exact(&mut self, bytes: &[u8]) -> bool {
		// SAFETY: the input slice is valid for byte copying and cannot alias this queue's pages.
		unsafe { self.write_exact_raw(bytes.as_ptr(), bytes.len()) }
	}

	pub(crate) fn peek(&self, output: &mut [u8]) -> usize {
		let count = output.len().min(self.length);
		// SAFETY: output is writable for count bytes and separate from the queue's private pages.
		unsafe { self.copy_out(output.as_mut_ptr(), count) };
		count
	}

	pub(crate) fn read(&mut self, output: &mut [u8]) -> usize {
		let count = output.len().min(self.length);
		// SAFETY: output is writable for count bytes and separate from the queue's private pages.
		unsafe { self.read_raw(output.as_mut_ptr(), count) };
		count
	}

	unsafe fn copy_out(&self, output: *mut u8, byte_count: usize) {
		debug_assert!(byte_count <= self.length);
		let mut page = self.head;
		let mut copied = 0;
		while copied < byte_count {
			debug_assert!(!page.is_null());
			// SAFETY: page_address identifies a live page in this queue's chain.
			let (start, end, next) = unsafe { ((*page).start, (*page).end, (*page).next) };
			let count = (end - start).min(byte_count - copied);
			debug_assert_ne!(count, 0);
			// SAFETY: the caller provides a writable, non-overlapping output range.
			unsafe {
				core::ptr::copy_nonoverlapping(
					Self::data_ptr(page).add(start),
					output.add(copied),
					count,
				);
			}
			copied += count;
			page = next;
		}
	}

	fn discard(&mut self, byte_count: usize) {
		debug_assert!(byte_count <= self.length);
		let mut remaining = byte_count;
		while remaining != 0 {
			let page_address = self.head;
			let page = page_address;
			// SAFETY: head identifies a live page containing queued bytes.
			let (start, end, next) = unsafe { ((*page).start, (*page).end, (*page).next) };
			let count = (end - start).min(remaining);
			let next_start = start + count;
			// SAFETY: page remains owned by this queue until all its bytes are consumed.
			unsafe { (*page).start = next_start };
			self.length -= count;
			remaining -= count;

			if next_start == end {
				if next.is_null() {
					self.head = core::ptr::null_mut();
					self.tail = core::ptr::null_mut();
				} else {
					self.head = next;
				}
				let _released =
					crate::kernel::memory::r#virtual::release(page_address as usize, PAGE_SIZE);
			}
		}
	}

	unsafe fn read_raw(&mut self, output: *mut u8, byte_count: usize) {
		debug_assert!(byte_count <= self.length);
		// SAFETY: the caller guarantees writable, non-overlapping output storage.
		unsafe { self.copy_out(output, byte_count) };
		self.discard(byte_count);
	}

	unsafe fn write_exact_raw(&mut self, input: *const u8, byte_count: usize) -> bool {
		let Some(new_length) = self.length.checked_add(byte_count) else {
			return false;
		};
		if byte_count == 0 {
			return true;
		}
		let Some(page_capacity) = Self::page_capacity() else {
			return false;
		};

		let previous_tail = self.tail;
		let tail_available = if previous_tail.is_null() {
			0
		} else {
			let tail = previous_tail;
			// SAFETY: previous_tail points to the live tail page owned by this queue.
			page_capacity - unsafe { (*tail).end }
		};
		let bytes_after_tail = byte_count.saturating_sub(tail_available);
		let pages_needed = bytes_after_tail / page_capacity
			+ usize::from(bytes_after_tail % page_capacity != 0);
		let mut new_head: *mut Page = core::ptr::null_mut();
		let mut new_tail: *mut Page = core::ptr::null_mut();
		for _ in 0..pages_needed {
			let Some(new_page) = Self::allocate_page() else {
				Self::release_chain(new_head);
				return false;
			};
			if new_tail.is_null() {
				new_head = new_page;
			} else {
				let tail = new_tail;
				// SAFETY: new_tail belongs to the temporary chain and new_page is newly allocated.
				unsafe { (*tail).next = new_page };
			}
			new_tail = new_page;
		}

		if !new_head.is_null() {
			if previous_tail.is_null() {
				self.head = new_head;
			} else {
				let tail = previous_tail;
				// SAFETY: tail belongs to this queue and new_head begins a live allocated chain.
				unsafe { (*tail).next = new_head };
			}
			self.tail = new_tail;
		}

		let mut page_address = if previous_tail.is_null() {
			new_head
		} else {
			previous_tail
		};
		let mut page_offset = if previous_tail.is_null() {
			0
		} else {
			let page = previous_tail;
			// SAFETY: previous_tail points to a live page owned by this queue.
			unsafe { (*page).end }
		};
		let mut copied = 0;
		while copied < byte_count {
			let page = page_address;
			let count = (page_capacity - page_offset).min(byte_count - copied);
			// SAFETY: the caller guarantees readable, non-overlapping input; reserved pages provide
			// enough writable space for the complete fixed-size record.
			unsafe {
				core::ptr::copy_nonoverlapping(
					input.add(copied),
					Self::data_ptr(page_address).add(page_offset),
					count,
				);
				(*page).end = page_offset + count;
			}
			copied += count;
			if copied < byte_count {
				// SAFETY: page_address is in the now-linked chain and another reserved page follows.
				page_address = unsafe { (*page).next };
				page_offset = 0;
			}
		}
		self.length = new_length;
		true
	}

	fn release_chain(mut page: *mut Page) {
		while !page.is_null() {
			// SAFETY: every page address in the chain refers to an allocated page.
			let next = unsafe { (*page).next };
			let _released = crate::kernel::memory::r#virtual::release(page as usize, PAGE_SIZE);
			page = next;
	}
	}

	pub(crate) fn clear(&mut self) {
		Self::release_chain(self.head);
		self.head = core::ptr::null_mut();
		self.tail = core::ptr::null_mut();
		self.length = 0;
	}
}

impl Drop for Queue {
	fn drop(&mut self) {
		self.clear();
	}
}

pub(crate) struct QueueTyped<T: Copy> {
	queue: Queue,
	marker: PhantomData<T>,
}

impl<T: Copy> QueueTyped<T> {
	pub(crate) const fn new() -> Self {
		Self {
			queue: Queue::new(),
			marker: PhantomData,
		}
	}

	pub(crate) fn len(&self) -> usize {
		let element_size = size_of::<T>();
		if element_size == 0 {
			0
		} else {
			self.queue.len() / element_size
		}
	}

	pub(crate) fn is_empty(&self) -> bool {
		self.queue.is_empty()
	}

	pub(crate) fn write(&mut self, value: T) -> Result<(), T> {
		let element_size = size_of::<T>();
		if element_size == 0 {
			return Err(value);
		}
		let input = (&value as *const T).cast::<u8>();
		// SAFETY: value remains alive for the copy, T is Copy, and this private queue only
		// reconstructs values of the same T through peek/read.
		if unsafe { self.queue.write_exact_raw(input, element_size) } {
			Ok(())
		} else {
			Err(value)
		}
	}

	pub(crate) fn peek(&self) -> Option<T> {
		let element_size = size_of::<T>();
		if element_size == 0 || self.queue.len() < element_size {
			return None;
		}
		let mut value = MaybeUninit::<T>::uninit();
		// SAFETY: this wrapper only queues exact byte copies of initialized T values.
		unsafe {
			self.queue
				.copy_out(value.as_mut_ptr().cast::<u8>(), element_size)
		};
		// SAFETY: all bytes of this T were copied from an existing value of the same type.
		Some(unsafe { value.assume_init() })
	}

	pub(crate) fn read(&mut self) -> Option<T> {
		let element_size = size_of::<T>();
		if element_size == 0 || self.queue.len() < element_size {
			return None;
		}
		let mut value = MaybeUninit::<T>::uninit();
		// SAFETY: this wrapper only queues exact byte copies of initialized T values.
		unsafe {
			self.queue
				.read_raw(value.as_mut_ptr().cast::<u8>(), element_size)
		};
		// SAFETY: all bytes of this T were copied from an existing value of the same type.
		Some(unsafe { value.assume_init() })
	}

	pub(crate) fn clear(&mut self) {
		self.queue.clear();
	}
}

#[cfg(test)]
#[path = "../../tests/kernel/lib/queue.rs"]
mod tests;
