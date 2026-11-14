/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::marker::PhantomData;
use core::mem::{align_of, size_of};
use core::ptr::NonNull;

const PAGE_SIZE: usize = crate::kernel::memory::r#virtual::PAGE_SIZE;
const NO_PAGE: usize = 0;

#[repr(C)]
struct Page {
	next: usize,
	start: usize,
	end: usize,
}

impl Page {
	const EMPTY: Self = Self {
		next: NO_PAGE,
		start: 0,
		end: 0,
	};
}

pub(crate) struct Queue<T> {
	head: usize,
	tail: usize,
	length: usize,
	marker: PhantomData<T>,
}

impl<T> Queue<T> {
	pub(crate) const fn new() -> Self {
		Self {
			head: NO_PAGE,
			tail: NO_PAGE,
			length: 0,
			marker: PhantomData,
		}
	}

	pub(crate) const fn len(&self) -> usize {
		self.length
	}

	pub(crate) const fn is_empty(&self) -> bool {
		self.length == 0
	}

	fn element_offset() -> usize {
		let alignment = align_of::<T>();
		(size_of::<Page>() + alignment - 1) & !(alignment - 1)
	}

	fn page_capacity() -> Option<usize> {
		let element_size = size_of::<T>();
		if element_size == 0 {
			return Some(usize::MAX);
		}
		if align_of::<T>() > PAGE_SIZE {
			return None;
		}
		PAGE_SIZE
			.checked_sub(Self::element_offset())
			.map(|available| available / element_size)
			.filter(|capacity| *capacity != 0)
	}

	fn element_ptr(page_address: usize, index: usize) -> *mut T {
		if size_of::<T>() == 0 {
			return NonNull::<T>::dangling().as_ptr();
		}
		let element_address = page_address + Self::element_offset() + index * size_of::<T>();
		element_address as *mut T
	}

	fn allocate_page() -> Option<usize> {
		let address = crate::kernel::memory::r#virtual::reserve(PAGE_SIZE, PAGE_SIZE, 2)?;
		let page = address as *mut Page;
		// SAFETY: reserve returns an aligned writable page that is exclusively owned by this queue.
		unsafe { page.write(Page::EMPTY) };
		Some(address)
	}

	pub(crate) fn try_push(&mut self, value: T) -> Result<(), T> {
		let Some(page_capacity) = Self::page_capacity() else {
			return Err(value);
		};
		let Some(new_length) = self.length.checked_add(1) else {
			return Err(value);
		};

		let tail_is_full = if self.tail == NO_PAGE {
			true
		} else {
			let tail = self.tail as *mut Page;
			// SAFETY: tail points to a live page owned by this queue.
			unsafe { (*tail).end == page_capacity }
		};

		if tail_is_full {
			let Some(new_page) = Self::allocate_page() else {
				return Err(value);
			};
			if self.tail == NO_PAGE {
				self.head = new_page;
			} else {
				let tail = self.tail as *mut Page;
				// SAFETY: tail is owned by this queue and new_page is a live allocated page.
				unsafe { (*tail).next = new_page };
			}
			self.tail = new_page;
		}

		let tail_address = self.tail;
		let tail = tail_address as *mut Page;
		// SAFETY: tail points to the live tail page owned by this queue.
		let index = unsafe { (*tail).end };
		debug_assert!(index < page_capacity);
		let element = Self::element_ptr(tail_address, index);
		// SAFETY: index is the next uninitialized slot in the owned tail page.
		unsafe {
			element.write(value);
			(*tail).end = index + 1;
		}
		self.length = new_length;
		Ok(())
	}

	pub(crate) fn pop(&mut self) -> Option<T> {
		if self.head == NO_PAGE {
			return None;
		}

		let page_address = self.head;
		let page = page_address as *mut Page;
		// SAFETY: head points to a live non-empty page owned by this queue.
		let (start, end, next) = unsafe { ((*page).start, (*page).end, (*page).next) };
		if start == end {
			return None;
		}

		let element = Self::element_ptr(page_address, start);
		// SAFETY: start identifies the first initialized slot in the head page.
		let value = unsafe { element.read() };
		let next_start = start + 1;
		// SAFETY: page remains owned until its initialized elements have been consumed.
		unsafe { (*page).start = next_start };
		self.length -= 1;

		if next_start == end {
			self.head = next;
			if next == NO_PAGE {
				self.tail = NO_PAGE;
			}
			let _released =
				crate::kernel::memory::r#virtual::release(page_address, PAGE_SIZE);
		}
		Some(value)
	}

	pub(crate) fn front(&self) -> Option<&T> {
		if self.head == NO_PAGE {
			return None;
		}

		let page_address = self.head;
		let page = page_address as *const Page;
		// SAFETY: head points to a live page owned by this queue.
		let (start, end) = unsafe { ((*page).start, (*page).end) };
		if start == end {
			return None;
		}

		let element = Self::element_ptr(page_address, start);
		// SAFETY: start identifies an initialized element, and the shared borrow prevents mutation.
		Some(unsafe { &*element })
	}

	pub(crate) fn clear(&mut self) {
		let mut page_address = self.head;
		while page_address != NO_PAGE {
			let page = page_address as *mut Page;
			// SAFETY: each address in the chain points to a live page owned by this queue.
			let (next, start, end) = unsafe { ((*page).next, (*page).start, (*page).end) };
			for index in start..end {
				let element = Self::element_ptr(page_address, index);
				// SAFETY: every slot from start to end is initialized and dropped exactly once.
				unsafe { core::ptr::drop_in_place(element) };
			}
			let _released = crate::kernel::memory::r#virtual::release(page_address, PAGE_SIZE);
			page_address = next;
		}
		self.head = NO_PAGE;
		self.tail = NO_PAGE;
		self.length = 0;
	}
}

impl<T> Drop for Queue<T> {
	fn drop(&mut self) {
		self.clear();
	}
}

#[cfg(test)]
#[path = "../../tests/kernel/lib/queue.rs"]
mod tests;
