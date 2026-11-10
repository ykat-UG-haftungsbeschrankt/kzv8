/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::marker::PhantomData;
use core::mem::{align_of, size_of, MaybeUninit};
use core::ptr::NonNull;

use crate::kernel::scheduler::sync::IrqSpinLock;

const PAGE_SIZE: usize = crate::kernel::memory::r#virtual::PAGE_SIZE;
const NO_SLOT: usize = usize::MAX;
const MAX_SLOTS_PER_PAGE: usize = PAGE_SIZE / size_of::<usize>();
const BITMAP_WORDS: usize =
    (MAX_SLOTS_PER_PAGE + u64::BITS as usize - 1) / u64::BITS as usize;

#[derive(Clone, Copy)]
pub(crate) struct SlabPage<const BITMAP_WORDS: usize> {
    address: usize,
    used: usize,
    free_hint: usize,
    allocated: [u64; BITMAP_WORDS],
}

impl<const BITMAP_WORDS: usize> SlabPage<BITMAP_WORDS> {
    pub(crate) const EMPTY: Self = Self {
        address: 0,
        used: 0,
        free_hint: 0,
        allocated: [0; BITMAP_WORDS],
    };

    pub(crate) const fn new(address: usize) -> Self {
        Self {
            address,
            ..Self::EMPTY
        }
    }

    pub(crate) const fn address(&self) -> usize {
        self.address
    }

    pub(crate) const fn used(&self) -> usize {
        self.used
    }

    pub(crate) fn capacity(slot_size: usize) -> Option<usize> {
        if slot_size == 0 || slot_size > PAGE_SIZE {
            return None;
        }
        let slot_count = PAGE_SIZE / slot_size;
        (slot_count <= BITMAP_WORDS.checked_mul(u64::BITS as usize)?).then_some(slot_count)
    }

    fn slot_index(&self, address: usize, slot_size: usize) -> Option<usize> {
        let slot_count = Self::capacity(slot_size)?;
        let offset = address.checked_sub(self.address)?;
        if self.address == 0 || offset >= PAGE_SIZE || offset % slot_size != 0 {
            return None;
        }
        let slot_index = offset / slot_size;
        (slot_index < slot_count).then_some(slot_index)
    }

    fn find_free_slot(&self, slot_count: usize, start: usize) -> Option<usize> {
        for offset in 0..slot_count {
            let slot_index = (start + offset) % slot_count;
            let word = slot_index / u64::BITS as usize;
            let mask = 1u64 << (slot_index % u64::BITS as usize);
            if self.allocated[word] & mask == 0 {
                return Some(slot_index);
            }
        }
        None
    }

    pub(crate) fn allocate(&mut self, slot_size: usize) -> Option<usize> {
        let slot_count = Self::capacity(slot_size)?;
        if self.address == 0 {
            return None;
        }
        let slot_index = self.find_free_slot(slot_count, self.free_hint)?;
        let word = slot_index / u64::BITS as usize;
        let mask = 1u64 << (slot_index % u64::BITS as usize);
        self.allocated[word] |= mask;
        self.used += 1;
        self.free_hint = self
            .find_free_slot(slot_count, slot_index + 1)
            .unwrap_or(slot_count);
        Some(self.address + slot_index * slot_size)
    }

    pub(crate) fn mark_allocated(&mut self, address: usize, slot_size: usize) -> bool {
        let Some(slot_count) = Self::capacity(slot_size) else {
            return false;
        };
        let Some(slot_index) = self.slot_index(address, slot_size) else {
            return false;
        };
        let word = slot_index / u64::BITS as usize;
        let mask = 1u64 << (slot_index % u64::BITS as usize);
        if self.allocated[word] & mask != 0 {
            return false;
        }
        self.allocated[word] |= mask;
        self.used += 1;
        if self.free_hint == slot_index {
            self.free_hint = self
                .find_free_slot(slot_count, slot_index + 1)
                .unwrap_or(slot_count);
        }
        true
    }

    pub(crate) fn deallocate(&mut self, address: usize, slot_size: usize) -> bool {
        let Some(slot_index) = self.slot_index(address, slot_size) else {
            return false;
        };
        let word = slot_index / u64::BITS as usize;
        let mask = 1u64 << (slot_index % u64::BITS as usize);
        if self.allocated[word] & mask == 0 {
            return false;
        }
        self.allocated[word] &= !mask;
        self.used -= 1;
        self.free_hint = self.free_hint.min(slot_index);
        true
    }

    pub(crate) fn is_allocated(&self, address: usize, slot_size: usize) -> bool {
        let Some(slot_index) = self.slot_index(address, slot_size) else {
            return false;
        };
        let word = slot_index / u64::BITS as usize;
        let mask = 1u64 << (slot_index % u64::BITS as usize);
        self.allocated[word] & mask != 0
    }
}

struct SlabState<const MAX_PAGES: usize> {
    pages: [SlabPage<BITMAP_WORDS>; MAX_PAGES],
    page_count: usize,
    free_head: usize,
}

impl<const MAX_PAGES: usize> SlabState<MAX_PAGES> {
    const fn new() -> Self {
        Self {
            pages: [SlabPage::EMPTY; MAX_PAGES],
            page_count: 0,
            free_head: NO_SLOT,
        }
    }

    fn page_index(&self, address: usize) -> Option<usize> {
        for index in 0..self.page_count {
            if self.pages[index].address() == address {
                return Some(index);
            }
        }
        None
    }
}

/// A fixed-size object cache backed by up to `MAX_PAGES` virtual pages.
pub(crate) struct Slab<T, const MAX_PAGES: usize = 16> {
    state: IrqSpinLock<SlabState<MAX_PAGES>>,
    marker: PhantomData<fn() -> T>,
}

impl<T, const MAX_PAGES: usize> Slab<T, MAX_PAGES> {
    pub(crate) const fn new() -> Self {
        Self {
            state: IrqSpinLock::new(SlabState::new()),
            marker: PhantomData,
        }
    }

    fn slot_layout() -> Option<(usize, usize)> {
        let alignment = if align_of::<T>() > align_of::<usize>() {
            align_of::<T>()
        } else {
            align_of::<usize>()
        };
        let size = if size_of::<T>() > size_of::<usize>() {
            size_of::<T>()
        } else {
            size_of::<usize>()
        };
        let stride = size.checked_add(alignment - 1)? & !(alignment - 1);
        if stride > PAGE_SIZE {
            return None;
        }
        Some((stride, PAGE_SIZE / stride))
    }

    pub(crate) fn allocate(&self) -> Option<NonNull<MaybeUninit<T>>> {
        let (stride, slot_count) = Self::slot_layout()?;
        let mut state = self.state.lock();
        if state.free_head == NO_SLOT && !Self::add_page(&mut state, stride, slot_count) {
            return None;
        }

        let address = state.free_head;
        if address == NO_SLOT {
            return None;
        }
        // SAFETY: free-list links are written into aligned slots of at least usize size.
        let next = unsafe { (address as *const usize).read() };
        let page_address = address & !(PAGE_SIZE - 1);
        let page_index = state.page_index(page_address)?;
        if !state.pages[page_index].mark_allocated(address, stride) {
            return None;
        }
        state.free_head = next;
        NonNull::new(address as *mut MaybeUninit<T>)
    }

    /// Returns a previously allocated slot to this slab.
    ///
    /// # Safety
    /// `pointer` must be a live allocation from this slab, and any initialized `T`
    /// in the slot must have been dropped. The slot must not be used after this call.
    pub(crate) unsafe fn deallocate(&self, pointer: NonNull<MaybeUninit<T>>) -> bool {
        let Some((stride, _)) = Self::slot_layout() else {
            return false;
        };
        let address = pointer.as_ptr() as usize;
        let page_address = address & !(PAGE_SIZE - 1);
        let mut state = self.state.lock();
        let Some(page_index) = state.page_index(page_address) else {
            return false;
        };
        if !state.pages[page_index].deallocate(address, stride) {
            return false;
        }

        // SAFETY: the caller guarantees the slot's T has been dropped, and the slot
        // has been validated as an allocated member of a retained slab page.
        unsafe { (address as *mut usize).write(state.free_head) };
        state.free_head = address;
        true
    }

    fn add_page(
        state: &mut SlabState<MAX_PAGES>,
        stride: usize,
        slot_count: usize,
    ) -> bool {
        if state.page_count >= MAX_PAGES {
            return false;
        }
        let Some(address) = crate::kernel::memory::r#virtual::reserve(PAGE_SIZE, PAGE_SIZE, 2)
        else {
            return false;
        };

        let page_index = state.page_count;
        state.pages[page_index] = SlabPage::new(address);
        state.page_count += 1;

        for slot_index in (0..slot_count).rev() {
            let slot_address = address + slot_index * stride;
            // SAFETY: the reserved page is page-aligned and each stride is usize-aligned.
            unsafe { (slot_address as *mut usize).write(state.free_head) };
            state.free_head = slot_address;
        }
        true
    }
}