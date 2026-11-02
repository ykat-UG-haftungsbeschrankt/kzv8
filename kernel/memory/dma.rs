/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::cell::UnsafeCell;
use core::sync::atomic::{fence, AtomicUsize, Ordering};



#[repr(C, align(4096))]
pub(crate) struct DmaArena {
    bytes: UnsafeCell<[u8; DMA_ARENA_SIZE]>,
    next: AtomicUsize,
}

// SAFETY: initialization runs before buffers are issued; `next` then reserves disjoint ranges.
unsafe impl Sync for DmaArena {}

pub(crate) struct DmaBuffer {
    pointer: *mut u8,
    length: usize,
}

impl DmaBuffer {
    pub(crate) const fn len(&self) -> usize {
        self.length
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub(crate) fn bus_address(&self) -> u64 {
        self.pointer as u64
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.pointer, self.length) }
    }
}

impl DmaArena {
    const DMA_ARENA_SIZE: usize = 512 * 1024;
    const fn new() -> Self {
        Self {
            bytes: UnsafeCell::new([0; Self::DMA_ARENA_SIZE]),
            next: AtomicUsize::new(0),
        }
    }

    fn get_instance() -> &'static Self {
        static ARENA: DmaArena = DmaArena::new();
        &ARENA
    }

    pub(crate) fn initialize() {
        Self::get_instance().next.store(0, Ordering::Release);
    }

    pub(crate) fn allocate(length: usize, alignment: usize) -> Option<DmaBuffer> {
        if length == 0 || !alignment.is_power_of_two() {
            return None;
        }
        let arena = Self::get_instance();
        let base = arena.bytes.get().cast::<u8>() as usize;
        let mut current = arena.next.load(Ordering::Relaxed);
        loop {
            let aligned = (base + current + alignment - 1) & !(alignment - 1);
            let end = aligned.checked_add(length)?;
            let next = end.checked_sub(base)?;
            if next > Self::DMA_ARENA_SIZE {
                return None;
            }
            match arena.next.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(DmaBuffer {
                        pointer: aligned as *mut u8,
                        length,
                    });
                }
                Err(updated) => current = updated,
            }
        }
    }

    pub(crate) fn sync_for_device() {
        fence(Ordering::SeqCst);
        crate::kernel::arch::dma_sync_for_device();
    }

    pub(crate) fn sync_for_cpu() {
        fence(Ordering::SeqCst);
        crate::kernel::arch::dma_sync_for_cpu();
    }

}