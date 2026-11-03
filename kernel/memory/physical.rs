/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::kernel::scheduler::sync::IrqSpinLock;



extern "C" {
    static __bss_end: u8;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Frame {
    address: usize,
}

impl Frame {
    pub(crate) const fn address(self) -> usize {
        self.address
    }
}

pub(crate) struct PhysicalAllocator {
    bitmap: [AtomicU64; PhysicalAllocator::BITMAP_WORDS],
    usable: [AtomicU64; PhysicalAllocator::BITMAP_WORDS],
    initialized: AtomicBool,
    frame_count: AtomicUsize,
    next_hint: AtomicUsize,
    memory_bytes: AtomicUsize,
    lock: IrqSpinLock<()>,
}

impl PhysicalAllocator {
    const PAGE_SIZE: usize = 4096;
    const MAX_FRAMES: usize = crate::kernel::memory::resources::MAX_PHYSICAL_ADDRESS / Self::PAGE_SIZE;
    const BITMAP_WORDS: usize = Self::MAX_FRAMES / u64::BITS as usize;

    const fn new() -> Self {
        Self {
            bitmap: [const { AtomicU64::new(u64::MAX) }; Self::BITMAP_WORDS],
            usable: [const { AtomicU64::new(0) }; Self::BITMAP_WORDS],
            initialized: AtomicBool::new(false),
            frame_count: AtomicUsize::new(0),
            next_hint: AtomicUsize::new(0),
            memory_bytes: AtomicUsize::new(0),
            lock: IrqSpinLock::new(()),
        }
    }
}

impl PhysicalAllocator {
    fn get_instance() -> &'static Self {
        static ALLOCATOR: PhysicalAllocator = PhysicalAllocator::new();
        &ALLOCATOR
    }

    pub(crate) fn initialize() -> bool {
        let allocator = Self::get_instance();
        let _guard = allocator.lock.lock();
        let image_end = core::ptr::addr_of!(__bss_end) as usize;
        for word in &allocator.bitmap {
            word.store(u64::MAX, Ordering::Relaxed);
        }
        for word in &allocator.usable {
            word.store(0, Ordering::Relaxed);
        }

        let mut frame_count = 0;
        let mut total_frames = 0;
        let mut next_hint = None;
        let memory_regions =
            crate::kernel::memory::resources::MemoryRegionRegistry::memory_regions();
        for region in memory_regions.iter() {
            let Some(region_end) = region.start.checked_add(region.length) else {
                continue;
            };
            let Some(start_address) = image_end
                .max(region.start)
                .checked_add(Self::PAGE_SIZE - 1)
                .map(|address| address & !(Self::PAGE_SIZE - 1))
            else {
                continue;
            };
            let end_address = region_end
                .min(crate::kernel::memory::resources::MAX_PHYSICAL_ADDRESS)
                & !(Self::PAGE_SIZE - 1);
            if start_address >= end_address {
                continue;
            }

            let start_frame = start_address / Self::PAGE_SIZE;
            let end_frame = end_address / Self::PAGE_SIZE;
            frame_count = frame_count.max(end_frame);
            next_hint = Some(next_hint.map_or(start_frame, |hint: usize| hint.min(start_frame)));
            for frame in start_frame..end_frame {
                let word_index = frame / u64::BITS as usize;
                let mask = 1u64 << (frame % u64::BITS as usize);
                let previous = allocator.usable[word_index].fetch_or(mask, Ordering::Relaxed);
                if previous & mask == 0 {
                    allocator.bitmap[word_index].fetch_and(!mask, Ordering::Relaxed);
                    total_frames += 1;
                }
            }
        }
        if total_frames == 0 {
            return false;
        }

        allocator.frame_count.store(frame_count, Ordering::Relaxed);
        allocator
            .next_hint
            .store(next_hint.unwrap_or(0), Ordering::Relaxed);
        allocator
            .memory_bytes
            .store(total_frames * Self::PAGE_SIZE, Ordering::Relaxed);
        allocator.initialized.store(true, Ordering::Release);
        true
    }

    pub(crate) fn allocate() -> Option<Frame> {
        let allocator = Self::get_instance();
        let _guard = allocator.lock.lock();
        if !allocator.initialized.load(Ordering::Acquire) {
            return None;
        }
        let count = allocator.frame_count.load(Ordering::Relaxed);
        if count == 0 {
            return None;
        }
        let hint = allocator.next_hint.load(Ordering::Relaxed).min(count - 1);

        for offset in 0..count {
            let frame = (hint + offset) % count;
            let word = &allocator.bitmap[frame / u64::BITS as usize];
            let mask = 1u64 << (frame % u64::BITS as usize);
            let mut current = word.load(Ordering::Relaxed);
            loop {
                if current & mask != 0 {
                    break;
                }
                match word.compare_exchange_weak(
                    current,
                    current | mask,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        allocator.next_hint.store(
                            if frame + 1 == count { 0 } else { frame + 1 },
                            Ordering::Relaxed,
                        );
                        return Some(Frame {
                            address: frame * Self::PAGE_SIZE,
                        });
                    }
                    Err(updated) => current = updated,
                }
            }
        }
        None
    }

    pub(crate) fn allocate_contiguous(length: usize, alignment: usize) -> Option<usize> {
        let allocator = Self::get_instance();
        let _guard = allocator.lock.lock();
        if !allocator.initialized.load(Ordering::Acquire)
            || length == 0
            || length % Self::PAGE_SIZE != 0
            || alignment < Self::PAGE_SIZE
            || !alignment.is_power_of_two()
        {
            return None;
        }

        let count = allocator.frame_count.load(Ordering::Relaxed);
        let pages = length / Self::PAGE_SIZE;
        if count == 0 || pages > count {
            return None;
        }

        let hint = allocator.next_hint.load(Ordering::Relaxed).min(count - 1);
        for offset in 0..count {
            let frame = (hint + offset) % count;
            let Some(end_frame) = frame.checked_add(pages) else {
                continue;
            };
            if end_frame > count {
                continue;
            }
            let Some(address) = frame.checked_mul(Self::PAGE_SIZE) else {
                continue;
            };
            if address & (alignment - 1) != 0 {
                continue;
            }

            let range_is_free = (frame..end_frame).all(|candidate| {
                let word_index = candidate / u64::BITS as usize;
                let mask = 1u64 << (candidate % u64::BITS as usize);
                allocator.usable[word_index].load(Ordering::Acquire) & mask != 0
                    && allocator.bitmap[word_index].load(Ordering::Acquire) & mask == 0
            });
            if !range_is_free {
                continue;
            }

            let mut reserved_end = frame;
            let mut collision = false;
            for candidate in frame..end_frame {
                let mask = 1u64 << (candidate % u64::BITS as usize);
                let previous = allocator.bitmap[candidate / u64::BITS as usize]
                    .fetch_or(mask, Ordering::AcqRel);
                if previous & mask != 0 {
                    collision = true;
                    break;
                }
                reserved_end = candidate + 1;
            }

            if collision {
                for candidate in frame..reserved_end {
                    let mask = 1u64 << (candidate % u64::BITS as usize);
                    allocator.bitmap[candidate / u64::BITS as usize]
                        .fetch_and(!mask, Ordering::Release);
                }
                continue;
            }

            allocator.next_hint.store(
                if end_frame == count { 0 } else { end_frame },
                Ordering::Relaxed,
            );
            return Some(address);
        }

        None
    }

    pub(crate) fn deallocate_contiguous(address: usize, length: usize) -> bool {
        let allocator = Self::get_instance();
        let _guard = allocator.lock.lock();
        if length == 0 || length % Self::PAGE_SIZE != 0 || address % Self::PAGE_SIZE != 0 {
            return false;
        }

        let count = allocator.frame_count.load(Ordering::Relaxed);
        let start_frame = address / Self::PAGE_SIZE;
        let pages = length / Self::PAGE_SIZE;
        let Some(end_frame) = start_frame.checked_add(pages) else {
            return false;
        };
        if end_frame > count {
            return false;
        }

        let range_is_allocated = (start_frame..end_frame).all(|frame| {
            let word_index = frame / u64::BITS as usize;
            let mask = 1u64 << (frame % u64::BITS as usize);
            allocator.usable[word_index].load(Ordering::Acquire) & mask != 0
                && allocator.bitmap[word_index].load(Ordering::Acquire) & mask != 0
        });
        if !range_is_allocated {
            return false;
        }

        for frame in start_frame..end_frame {
            let mask = 1u64 << (frame % u64::BITS as usize);
            allocator.bitmap[frame / u64::BITS as usize].fetch_and(!mask, Ordering::Release);
        }
        allocator.next_hint.store(start_frame, Ordering::Relaxed);
        true
    }

    pub(crate) fn deallocate(frame: Frame) -> bool {
        let allocator = Self::get_instance();
        let _guard = allocator.lock.lock();
        if frame.address % Self::PAGE_SIZE != 0
            || frame.address >= crate::kernel::memory::resources::MAX_PHYSICAL_ADDRESS
        {
            return false;
        }
        let index = frame.address / Self::PAGE_SIZE;
        let count = allocator.frame_count.load(Ordering::Relaxed);
        if index >= count {
            return false;
        }
        let mask = 1u64 << (index % u64::BITS as usize);
        let word_index = index / u64::BITS as usize;
        if allocator.usable[word_index].load(Ordering::Acquire) & mask == 0 {
            return false;
        }
        let previous = allocator.bitmap[word_index].fetch_and(!mask, Ordering::AcqRel);
        if previous & mask == 0 {
            return false;
        }
        allocator.next_hint.store(index, Ordering::Relaxed);
        true
    }

    pub(crate) fn free_frames() -> usize {
        let allocator = Self::get_instance();
        let _guard = allocator.lock.lock();
        let count = allocator.frame_count.load(Ordering::Relaxed);
        (0..count)
            .filter(|frame| {
                allocator.bitmap[*frame / u64::BITS as usize].load(Ordering::Relaxed)
                    & (1u64 << (*frame % u64::BITS as usize))
                    == 0
            })
            .count()
    }

    pub(crate) fn memory_bytes() -> usize {
        let allocator = Self::get_instance();
        allocator.memory_bytes.load(Ordering::Acquire)
    }
}
