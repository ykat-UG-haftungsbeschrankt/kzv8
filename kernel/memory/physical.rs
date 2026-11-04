/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::mem::{align_of, size_of};

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
    state: IrqSpinLock<AllocatorState>,
}

#[derive(Clone, Copy)]
struct AllocatorRegion {
    start_frame: usize,
    frame_count: usize,
    bitmap_start: usize,
}

struct AllocatorState {
    initialized: bool,
    regions: usize,
    region_count: usize,
    bitmap_address: usize,
    bitmap_words: usize,
    frame_count: usize,
    next_hint: usize,
    memory_bytes: usize,
}

impl AllocatorState {
    const fn new() -> Self {
        Self {
            initialized: false,
            regions: 0,
            region_count: 0,
            bitmap_address: 0,
            bitmap_words: 0,
            frame_count: 0,
            next_hint: 0,
            memory_bytes: 0,
        }
    }

    fn regions(&self) -> &[AllocatorRegion] {
        if self.region_count == 0 {
            return &[];
        }
        // SAFETY: the boot allocator keeps this initialized region table allocated permanently.
        unsafe {
            core::slice::from_raw_parts(self.regions as *const AllocatorRegion, self.region_count)
        }
    }

    fn is_allocated(&self, index: usize) -> bool {
        let word = (self.bitmap_address as *const u64).wrapping_add(index / u64::BITS as usize);
        let mask = 1u64 << (index % u64::BITS as usize);
        // SAFETY: the bitmap is initialized in mapped RAM, and callers hold the allocator lock.
        unsafe { word.read() & mask != 0 }
    }

    fn set_allocated(&mut self, index: usize, allocated: bool) {
        let word = (self.bitmap_address as *mut u64).wrapping_add(index / u64::BITS as usize);
        let mask = 1u64 << (index % u64::BITS as usize);
        // SAFETY: the bitmap is initialized in mapped RAM, and callers hold the allocator lock.
        unsafe {
            let current = word.read();
            word.write(if allocated {
                current | mask
            } else {
                current & !mask
            });
        }
    }

    fn address_for_index(&self, index: usize) -> Option<usize> {
        for region in self.regions() {
            let end = region.bitmap_start.checked_add(region.frame_count)?;
            if (region.bitmap_start..end).contains(&index) {
                return region
                    .start_frame
                    .checked_add(index - region.bitmap_start)?
                    .checked_mul(PhysicalAllocator::PAGE_SIZE);
            }
        }
        None
    }

    fn index_for_address(&self, address: usize) -> Option<usize> {
        if address % PhysicalAllocator::PAGE_SIZE != 0 {
            return None;
        }
        let frame = address / PhysicalAllocator::PAGE_SIZE;
        for region in self.regions() {
            let end = region.start_frame.checked_add(region.frame_count)?;
            if (region.start_frame..end).contains(&frame) {
                return region.bitmap_start.checked_add(frame - region.start_frame);
            }
        }
        None
    }
}

impl PhysicalAllocator {
    const PAGE_SIZE: usize = 4096;

    const fn new() -> Self {
        Self {
            state: IrqSpinLock::new(AllocatorState::new()),
        }
    }

    fn get_instance() -> &'static Self {
        static ALLOCATOR: PhysicalAllocator = PhysicalAllocator::new();
        &ALLOCATOR
    }

    pub(crate) fn initialize() -> bool {
        let allocator = Self::get_instance();
        let image_end = core::ptr::addr_of!(__bss_end) as usize;
        let memory_regions = crate::kernel::memory::memblock::MemBlock::memory_regions();
        let Some(region_bytes) = memory_regions
            .as_slice()
            .len()
            .checked_mul(size_of::<AllocatorRegion>())
        else {
            return false;
        };
        let Some(regions_address) =
            crate::kernel::memory::memblock::MemBlock::allocate_early(
                region_bytes,
                align_of::<AllocatorRegion>(),
            )
        else {
            return false;
        };
        let regions = regions_address as *mut AllocatorRegion;
        let mut region_count = 0;
        let mut total_frames = 0usize;
        let mut memory_bytes = 0usize;
        let physical_limit = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::physical_address_limit() as usize;

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
            let end_address = region_end.min(physical_limit) & !(Self::PAGE_SIZE - 1);
            if start_address >= end_address {
                continue;
            }

            let start_frame = start_address / Self::PAGE_SIZE;
            let frame_count = (end_address - start_address) / Self::PAGE_SIZE;
            // SAFETY: `regions_address` reserves space for every memory region.
            unsafe {
                regions.add(region_count).write(AllocatorRegion {
                    start_frame,
                    frame_count,
                    bitmap_start: total_frames,
                });
            }
            region_count += 1;
            let Some(new_total_frames) = total_frames.checked_add(frame_count) else {
                return false;
            };
            total_frames = new_total_frames;
            let Some(region_bytes) = frame_count.checked_mul(Self::PAGE_SIZE) else {
                return false;
            };
            let Some(new_memory_bytes) = memory_bytes.checked_add(region_bytes)
            else {
                return false;
            };
            memory_bytes = new_memory_bytes;
        }
        if total_frames == 0 {
            return false;
        }

        let Some(bitmap_words) = total_frames.checked_add(u64::BITS as usize - 1)
            .map(|frames| frames / u64::BITS as usize)
        else {
            return false;
        };
        let Some(bitmap_bytes) = bitmap_words.checked_mul(size_of::<u64>()) else {
            return false;
        };
        let Some(bitmap_length) = bitmap_bytes
            .checked_add(Self::PAGE_SIZE - 1)
            .map(|length| length & !(Self::PAGE_SIZE - 1))
        else {
            return false;
        };
        let Some(bitmap_address) =
            crate::kernel::memory::memblock::MemBlock::allocate_early(bitmap_length, Self::PAGE_SIZE)
        else {
            return false;
        };

        // SAFETY: the selected bitmap range is page-aligned, lies in usable RAM, and is identity-mapped.
        let bitmap = unsafe {
            core::slice::from_raw_parts_mut(bitmap_address as *mut u64, bitmap_words)
        };
        bitmap.fill(0);
        let remaining_bits = total_frames % u64::BITS as usize;
        if remaining_bits != 0 {
            bitmap[bitmap_words - 1] = u64::MAX << remaining_bits;
        }

        // Reserve boot-data ranges and all early metadata allocations.
        let region_table = unsafe { core::slice::from_raw_parts(regions, region_count) };
        crate::kernel::memory::memblock::MemBlock::for_each_reserved_range(
            |allocation_start, allocation_length| {
                let Some(allocation_end) = allocation_start.checked_add(allocation_length) else {
                    return;
                };
                let allocation_start_frame = allocation_start / Self::PAGE_SIZE;
                let Some(allocation_end_frame) = allocation_end
                    .checked_add(Self::PAGE_SIZE - 1)
                    .map(|end| end / Self::PAGE_SIZE)
                else {
                    return;
                };
                for region in region_table {
                    let Some(region_end_frame) = region.start_frame.checked_add(region.frame_count)
                    else {
                        continue;
                    };
                    let start_frame = allocation_start_frame.max(region.start_frame);
                    let end_frame = allocation_end_frame.min(region_end_frame);
                    for frame in start_frame..end_frame {
                        let index = region.bitmap_start + (frame - region.start_frame);
                        bitmap[index / u64::BITS as usize] |=
                            1u64 << (index % u64::BITS as usize);
                    }
                }
            },
        );

        let mut state = allocator.state.lock();
        state.initialized = false;
        state.regions = regions_address;
        state.region_count = region_count;
        state.bitmap_address = bitmap_address;
        state.bitmap_words = bitmap_words;
        state.frame_count = total_frames;
        state.next_hint = 0;
        state.memory_bytes = memory_bytes;
        state.initialized = true;
        true
    }

    pub(crate) fn allocate() -> Option<Frame> {
        let allocator = Self::get_instance();
        let mut state = allocator.state.lock();
        if !state.initialized {
            return None;
        }
        let count = state.frame_count;
        if count == 0 {
            return None;
        }
        let hint = state.next_hint.min(count - 1);

        for offset in 0..count {
            let frame = hint.wrapping_add(offset) % count;
            if !state.is_allocated(frame) {
                let address = state.address_for_index(frame)?;
                state.set_allocated(frame, true);
                state.next_hint = if frame + 1 == count { 0 } else { frame + 1 };
                return Some(Frame { address });
            }
        }
        None
    }

    pub(crate) fn allocate_contiguous(length: usize, alignment: usize) -> Option<usize> {
        let allocator = Self::get_instance();
        let mut state = allocator.state.lock();
        if !state.initialized
            || length == 0
            || length % Self::PAGE_SIZE != 0
            || alignment < Self::PAGE_SIZE
            || !alignment.is_power_of_two()
        {
            return None;
        }

        let pages = length / Self::PAGE_SIZE;
        let count = state.frame_count;
        if count == 0 || pages > count {
            return None;
        }

        for region_index in 0..state.region_count {
            let region = state.regions()[region_index];
            if pages > region.frame_count {
                continue;
            }
            let region_end = region.bitmap_start + region.frame_count;
            let mut free_run = 0usize;
            for end in region.bitmap_start..region_end {
                if state.is_allocated(end) {
                    free_run = 0;
                    continue;
                }

                free_run += 1;
                if free_run < pages {
                    continue;
                }
                let end_exclusive = end + 1;
                let start = end_exclusive - pages;
                let frame_address = region.start_frame + start - region.bitmap_start;
                let address = frame_address.checked_mul(Self::PAGE_SIZE)?;
                if address & (alignment - 1) != 0 {
                    continue;
                }

                for frame in start..end_exclusive {
                    state.set_allocated(frame, true);
                }
                state.next_hint = end_exclusive % count;
                return Some(address);
            }
        }

        None
    }

    pub(crate) fn deallocate_contiguous(address: usize, length: usize) -> bool {
        let allocator = Self::get_instance();
        let mut state = allocator.state.lock();
        if length == 0 || length % Self::PAGE_SIZE != 0 || address % Self::PAGE_SIZE != 0 {
            return false;
        }

        if !state.initialized {
            return false;
        }
        let Some(start_frame) = state.index_for_address(address) else {
            return false;
        };
        let pages = length / Self::PAGE_SIZE;
        let Some(end_frame) = start_frame.checked_add(pages) else {
            return false;
        };
        let Some(region) = state.regions().iter().find(|region| {
            let end = region.bitmap_start + region.frame_count;
            start_frame >= region.bitmap_start && end_frame <= end
        }) else {
            return false;
        };
        let _ = region;

        let range_is_allocated = (start_frame..end_frame).all(|frame| state.is_allocated(frame));
        if !range_is_allocated {
            return false;
        }

        for frame in start_frame..end_frame {
            state.set_allocated(frame, false);
        }
        state.next_hint = start_frame;
        true
    }

    pub(crate) fn deallocate(frame: Frame) -> bool {
        let allocator = Self::get_instance();
        let mut state = allocator.state.lock();
        if !state.initialized {
            return false;
        }
        let Some(index) = state.index_for_address(frame.address) else {
            return false;
        };
        if !state.is_allocated(index) {
            return false;
        }
        state.set_allocated(index, false);
        state.next_hint = index;
        true
    }

    pub(crate) fn free_frames() -> usize {
        let allocator = Self::get_instance();
        let state = allocator.state.lock();
        if !state.initialized {
            return 0;
        }
        let mut free = 0usize;
        for word_index in 0..state.bitmap_words {
            let valid_bits = (state.frame_count - word_index * u64::BITS as usize)
                .min(u64::BITS as usize);
            let valid_mask = if valid_bits == u64::BITS as usize {
                u64::MAX
            } else {
                (1u64 << valid_bits) - 1
            };
            let word = (state.bitmap_address as *const u64).wrapping_add(word_index);
            // SAFETY: the bitmap is initialized in mapped RAM, and this lock protects access.
            let allocated = unsafe { word.read() };
            free += ((!allocated) & valid_mask).count_ones() as usize;
        }
        free
    }

    pub(crate) fn memory_bytes() -> usize {
        let allocator = Self::get_instance();
        let state = allocator.state.lock();
        if state.initialized {
            state.memory_bytes
        } else {
            0
        }
    }
}
