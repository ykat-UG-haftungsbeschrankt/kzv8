/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::ptr::null_mut;

use crate::kernel::scheduler::sync::IrqSpinLock;

#[derive(Clone, Copy)]
struct Slab {
    page: usize,
    class_index: usize,
    used: usize,
    next: usize,
    bitmap: [u64; 4],
    active: bool,
}

impl Slab {
    const EMPTY: Self = Self {
        page: 0,
        class_index: 0,
        used: 0,
        next: Heap::NO_INDEX,
        bitmap: [0; 4],
        active: false,
    };
}

#[derive(Clone, Copy)]
struct LargeAllocation {
    address: usize,
    length: usize,
    active: bool,
}

impl LargeAllocation {
    const EMPTY: Self = Self {
        address: 0,
        length: 0,
        active: false,
    };
}

struct HeapState {
    initialized: bool,
    slabs: [Slab; Heap::MAX_SLABS],
    class_heads: [usize; Heap::CLASS_SIZES.len()],
    large_allocations: [LargeAllocation; Heap::MAX_LARGE_ALLOCATIONS],
}

impl HeapState {
    const fn new() -> Self {
        Self {
            initialized: false,
            slabs: [Slab::EMPTY; Heap::MAX_SLABS],
            class_heads: [Heap::NO_INDEX; Heap::CLASS_SIZES.len()],
            large_allocations: [LargeAllocation::EMPTY; Heap::MAX_LARGE_ALLOCATIONS],
        }
    }
}

pub(crate) struct Heap {
    state: IrqSpinLock<HeapState>,
}

#[global_allocator]
static GLOBAL_ALLOCATOR: Heap = Heap::new();

impl Heap {
    const PAGE_SIZE: usize = crate::kernel::memory::virtual_memory::PAGE_SIZE;
    const CLASS_SIZES: [usize; 8] = [16, 32, 64, 128, 256, 512, 1024, 2048];
    const MAX_SLABS: usize = 8192;
    const MAX_LARGE_ALLOCATIONS: usize = 512;
    const NO_INDEX: usize = usize::MAX;

    const fn new() -> Self {
        Self {
            state: IrqSpinLock::new(HeapState::new()),
        }
    }

    fn get_instance() -> &'static Self {
        &GLOBAL_ALLOCATOR
    }

    fn allocate(&self, heap: &mut HeapState, layout: Layout) -> *mut u8 {
        if !heap.initialized {
            return null_mut();
        }
        let size = layout.size().max(1);
        let required = size.max(layout.align());
        if let Some(class_index) = Self::CLASS_SIZES
            .iter()
            .position(|class| required <= *class)
        {
            return self.allocate_small(heap, class_index);
        }
        self.allocate_large(heap, size, layout.align())
    }

    fn allocate_small(&self, heap: &mut HeapState, class_index: usize) -> *mut u8 {
        let class_size = Self::CLASS_SIZES[class_index];
        let slot_count = Self::PAGE_SIZE / class_size;
        let mut slab_index = heap.class_heads[class_index];

        while slab_index != Self::NO_INDEX {
            if heap.slabs[slab_index].used < slot_count {
                for slot in 0..slot_count {
                    let word = slot / u64::BITS as usize;
                    let mask = 1u64 << (slot % u64::BITS as usize);
                    let bitmap = heap.slabs[slab_index].bitmap[word];
                    if bitmap & mask == 0 {
                        let slab = &mut heap.slabs[slab_index];
                        slab.bitmap[word] = bitmap | mask;
                        slab.used += 1;
                        return (slab.page + slot * class_size) as *mut u8;
                    }
                }
            }
            slab_index = heap.slabs[slab_index].next;
        }

        let Some(metadata_slot) = heap.slabs.iter().position(|slab| !slab.active) else {
            return null_mut();
        };
        let Some(page) = crate::kernel::memory::virtual_memory::reserve(
            Self::PAGE_SIZE,
            Self::PAGE_SIZE,
            2,
        )
        else {
            return null_mut();
        };
        let head = heap.class_heads[class_index];
        heap.slabs[metadata_slot] = Slab {
            page,
            class_index,
            used: 1,
            next: head,
            bitmap: [1, 0, 0, 0],
            active: true,
        };
        heap.class_heads[class_index] = metadata_slot;
        page as *mut u8
    }

    fn rounded_length(length: usize) -> Option<usize> {
        length
            .checked_add(Self::PAGE_SIZE - 1)
            .map(|value| value & !(Self::PAGE_SIZE - 1))
    }

    fn allocate_large(&self, heap: &mut HeapState, size: usize, alignment: usize) -> *mut u8 {
        let Some(metadata_slot) = heap
            .large_allocations
            .iter()
            .position(|allocation| !allocation.active)
        else {
            return null_mut();
        };
        let Some(length) = Self::rounded_length(size) else {
            return null_mut();
        };
        let alignment = alignment.max(Self::PAGE_SIZE);
        let Some(address) = crate::kernel::memory::virtual_memory::reserve(length, alignment, 2)
        else {
            return null_mut();
        };
        heap.large_allocations[metadata_slot] = LargeAllocation {
            address,
            length,
            active: true,
        };
        address as *mut u8
    }

    fn deallocate(&self, heap: &mut HeapState, pointer: *mut u8) {
        if pointer.is_null() || !heap.initialized {
            return;
        }
        let address = pointer as usize;
        if let Some(index) = heap
            .large_allocations
            .iter()
            .position(|allocation| allocation.active && allocation.address == address)
        {
            let allocation = heap.large_allocations[index];
            if crate::kernel::memory::virtual_memory::release(allocation.address, allocation.length)
            {
                heap.large_allocations[index] = LargeAllocation::EMPTY;
            }
            return;
        }

        let page = address & !(Self::PAGE_SIZE - 1);
        let Some(index) = heap
            .slabs
            .iter()
            .position(|slab| slab.active && slab.page == page)
        else {
            return;
        };
        let slab = heap.slabs[index];
        let class_size = Self::CLASS_SIZES[slab.class_index];
        let offset = address - page;
        if offset % class_size != 0 {
            return;
        }
        let slot = offset / class_size;
        let slot_count = Self::PAGE_SIZE / class_size;
        if slot >= slot_count {
            return;
        }
        let word = slot / u64::BITS as usize;
        let mask = 1u64 << (slot % u64::BITS as usize);
        if slab.bitmap[word] & mask == 0 {
            return;
        }
        let class_index = slab.class_index;
        let unused = {
            let slab = &mut heap.slabs[index];
            slab.bitmap[word] &= !mask;
            slab.used -= 1;
            slab.used == 0
        };
        if unused && crate::kernel::memory::virtual_memory::release(page, Self::PAGE_SIZE) {
            let mut previous = Self::NO_INDEX;
            let mut current = heap.class_heads[class_index];
            while current != Self::NO_INDEX {
                let next = heap.slabs[current].next;
                if current == index {
                    if previous == Self::NO_INDEX {
                        heap.class_heads[class_index] = next;
                    } else {
                        heap.slabs[previous].next = next;
                    }
                    heap.slabs[index] = Slab::EMPTY;
                    break;
                }
                previous = current;
                current = next;
            }
        }
    }

    fn allocation_capacity(&self, heap: &HeapState, pointer: *const u8) -> Option<usize> {
        if pointer.is_null() || !heap.initialized {
            return None;
        }
        let address = pointer as usize;
        if let Some(allocation) = heap
            .large_allocations
            .iter()
            .find(|allocation| allocation.active && allocation.address == address)
        {
            return Some(allocation.length);
        }

        let page = address & !(Self::PAGE_SIZE - 1);
        let slab = heap
            .slabs
            .iter()
            .find(|slab| slab.active && slab.page == page)?;
        let class_size = Self::CLASS_SIZES[slab.class_index];
        let offset = address - page;
        if offset % class_size != 0 {
            return None;
        }
        let slot = offset / class_size;
        if slot >= Self::PAGE_SIZE / class_size {
            return None;
        }
        let mask = 1u64 << (slot % u64::BITS as usize);
        (slab.bitmap[slot / u64::BITS as usize] & mask != 0).then_some(class_size)
    }

    fn remaining(&self, heap: &HeapState) -> usize {
        let mut total = 0usize;
        for slab in &heap.slabs {
            if slab.active {
                let class_size = Self::CLASS_SIZES[slab.class_index];
                let slots = Self::PAGE_SIZE / class_size;
                total = total.saturating_add((slots - slab.used) * class_size);
            }
        }
        total
    }

    pub(crate) fn initialize() {
        let mut heap = Self::get_instance().state.lock();
        heap.initialized = false;
        heap.slabs.fill(Slab::EMPTY);
        heap.class_heads.fill(Self::NO_INDEX);
        heap.large_allocations.fill(LargeAllocation::EMPTY);
        heap.initialized = true;
    }
}

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.allocate(&mut self.state.lock(), layout)
    }

    unsafe fn dealloc(&self, pointer: *mut u8, _layout: Layout) {
        self.deallocate(&mut self.state.lock(), pointer)
    }
}
