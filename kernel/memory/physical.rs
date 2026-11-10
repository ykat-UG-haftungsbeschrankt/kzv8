/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

/*
 * Range-header physical allocator prototype, currently selected as memory::physical.
 *
 * Each usable firmware range stores a PhysicalMemory header in its first page.
 * Subsequent chunk-metadata pages each describe up to 1,916 following frames.
 * Two reserved virtual scratch pages let the allocator access that metadata
 * without relying on a low physical identity map.
 */

use core::mem::size_of;

use crate::kernel::arch::traits::MemoryArchitecture as _;
use crate::kernel::arch::MemoryArchitecture;
use crate::kernel::lib::alignment::align_up;
use crate::kernel::lib::address::{AddressTranslator, PhysAddr};
use crate::kernel::lib::bitfield::BitField;
use crate::kernel::lib::list::CircularListLinks;
use crate::kernel::lib::array::Array;
use crate::kernel::scheduler::sync::IrqSpinLock;

const PAGE_SIZE: usize = 4096;
const NO_LINK: usize = usize::MAX;
const MAX_FRAMES_PER_CHUNK: usize = 1916;
const BITMAP_WORDS: usize = MAX_FRAMES_PER_CHUNK.div_ceil(u64::BITS as usize);

extern "C" {
    static __bss_end: u8;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
enum ChunkList {
    Detached,
    Available,
    Full,
}

#[repr(C, align(4096))]
struct PhysicalChunk {
    links: CircularListLinks<usize>,
    page_count: u16,
    virgin_cursor: u16,
    list: ChunkList,
    free_stack: Array<u16, MAX_FRAMES_PER_CHUNK>,
    allocated: BitField<BITMAP_WORDS>,
}

const _: [(); PAGE_SIZE] = [(); size_of::<PhysicalChunk>()];

impl PhysicalChunk {
    fn new(page_count: u16) -> Self {
        Self {
            links: CircularListLinks::new(NO_LINK, NO_LINK),
            page_count,
            virgin_cursor: 0,
            list: ChunkList::Detached,
            free_stack: Array::new(0),
            allocated: BitField::new(),
        }
    }

    fn free_pages(&self) -> usize {
        usize::from(self.page_count.saturating_sub(self.virgin_cursor))
            + self.free_stack.len()
    }
}

#[repr(C, align(4096))]
struct PhysicalMemory {
    base: PhysAddr,
    length: usize,
    addresses: AddressTranslator,
    range_links: CircularListLinks<usize>,
    virgin_chunk_cursor: usize,
    available_head: usize,
    full_head: usize,
    chunk_window: usize,
}

const _: [(); PAGE_SIZE] = [(); size_of::<PhysicalMemory>()];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Frame {
    address: usize,
    range_base: PhysAddr,
    chunk_offset: usize,
    page_index: u16,
}

impl Frame {
    pub(crate) const fn address(self) -> usize {
        self.address
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FrameRun {
    address: PhysAddr,
    range_base: PhysAddr,
    chunk_offset: usize,
    start_page: u16,
    page_count: u16,
}

impl PhysicalMemory {
    fn new(base: PhysAddr, length: usize, chunk_window: usize) -> Option<Self> {
        if base.as_usize() == 0
            || base.as_usize() % PAGE_SIZE != 0
            || length < 3 * PAGE_SIZE
            || length % PAGE_SIZE != 0
        {
            return None;
        }
        Some(Self {
            base,
            length,
            addresses: AddressTranslator::new(base, length)?,
            range_links: CircularListLinks::new(NO_LINK, NO_LINK),
            virgin_chunk_cursor: PAGE_SIZE,
            available_head: NO_LINK,
            full_head: NO_LINK,
            chunk_window,
        })
    }

    fn usable_frame_count(&self) -> Option<usize> {
        let remaining_pages = self.length.checked_div(PAGE_SIZE)?.checked_sub(1)?;
        let pages_per_full_chunk = MAX_FRAMES_PER_CHUNK + 1;
        let full_chunks = remaining_pages / pages_per_full_chunk;
        let trailing_pages = remaining_pages % pages_per_full_chunk;
        full_chunks
            .checked_mul(MAX_FRAMES_PER_CHUNK)?
            .checked_add(trailing_pages.saturating_sub(1).min(MAX_FRAMES_PER_CHUNK))
    }

    fn with_chunk_mut<R>(
        &mut self,
        chunk_offset: usize,
        operation: impl for<'a> FnOnce(&'a mut PhysicalChunk) -> R,
    ) -> Option<R> {
        let chunk_end = chunk_offset.checked_add(PAGE_SIZE)?;
        if chunk_offset < PAGE_SIZE || chunk_offset % PAGE_SIZE != 0 || chunk_end > self.length {
            return None;
        }
        let chunk_address = self.addresses.translate(chunk_offset)?;
        with_mapped_page(self.chunk_window, chunk_address, |mapped_address| {
            // SAFETY: the scratch slot maps this dedicated, page-aligned chunk page.
            unsafe { operation(&mut *(mapped_address as *mut PhysicalChunk)) }
        })
    }

    fn list_head(&self, list: ChunkList) -> usize {
        match list {
            ChunkList::Available => self.available_head,
            ChunkList::Full => self.full_head,
            ChunkList::Detached => NO_LINK,
        }
    }

    fn set_list_head(&mut self, list: ChunkList, offset: usize) {
        match list {
            ChunkList::Available => self.available_head = offset,
            ChunkList::Full => self.full_head = offset,
            ChunkList::Detached => {}
        }
    }

    fn insert_chunk(&mut self, offset: usize, list: ChunkList) -> bool {
        let Some(current_list) = self.with_chunk_mut(offset, |chunk| chunk.list) else {
            return false;
        };
        if list == ChunkList::Detached || current_list != ChunkList::Detached {
            return false;
        }

        let head = self.list_head(list);
        if head == NO_LINK {
            if self
                .with_chunk_mut(offset, |chunk| {
                    chunk.links.set_previous(offset);
                    chunk.links.set_next(offset);
                    chunk.list = list;
                })
                .is_none()
            {
                return false;
            }
            self.set_list_head(list, offset);
            return true;
        }

        let Some(tail) = self.with_chunk_mut(head, |chunk| chunk.links.previous()) else {
            return false;
        };
        if self
            .with_chunk_mut(offset, |chunk| {
                chunk.links.set_previous(tail);
                chunk.links.set_next(head);
                chunk.list = list;
            })
            .is_none()
            || self
                .with_chunk_mut(tail, |chunk| chunk.links.set_next(offset))
                .is_none()
            || self
                .with_chunk_mut(head, |chunk| chunk.links.set_previous(offset))
                .is_none()
        {
            return false;
        }
        self.set_list_head(list, offset);
        true
    }

    fn remove_chunk(&mut self, offset: usize) -> bool {
        let Some((previous, next, list)) = self.with_chunk_mut(offset, |chunk| {
            (chunk.links.previous(), chunk.links.next(), chunk.list)
        }) else {
            return false;
        };
        if list == ChunkList::Detached {
            return false;
        }

        if previous == offset && next == offset {
            self.set_list_head(list, NO_LINK);
        } else {
            if self
                .with_chunk_mut(previous, |chunk| chunk.links.set_next(next))
                .is_none()
                || self
                    .with_chunk_mut(next, |chunk| chunk.links.set_previous(previous))
                    .is_none()
            {
                return false;
            }
            if self.list_head(list) == offset {
                self.set_list_head(list, next);
            }
        }
        self.with_chunk_mut(offset, |chunk| {
            chunk.links.reset(NO_LINK);
            chunk.list = ChunkList::Detached;
        });
        true
    }

    fn move_chunk(&mut self, offset: usize, list: ChunkList) -> bool {
        let Some(current_list) = self.with_chunk_mut(offset, |chunk| chunk.list) else {
            return false;
        };
        if current_list == list {
            return true;
        }
        if current_list != ChunkList::Detached && !self.remove_chunk(offset) {
            return false;
        }
        self.insert_chunk(offset, list)
    }

    fn activate_next_chunk(&mut self) -> Option<usize> {
        let chunk_offset = self.virgin_chunk_cursor;
        let data_offset = chunk_offset.checked_add(PAGE_SIZE)?;
        if data_offset >= self.length {
            return None;
        }
        let page_count = ((self.length - data_offset) / PAGE_SIZE).min(MAX_FRAMES_PER_CHUNK);
        if page_count == 0 {
            return None;
        }
        let chunk_address = self.addresses.translate(chunk_offset)?;
        let next_cursor = data_offset.checked_add(page_count.checked_mul(PAGE_SIZE)?)?;
        let Some(()) = with_mapped_page(self.chunk_window, chunk_address, |mapped_address| {
            // SAFETY: this cursor page has never been used and is reserved for metadata.
            unsafe { (mapped_address as *mut PhysicalChunk).write(PhysicalChunk::new(page_count as u16)) };
        }) else {
            return None;
        };
        if !self.insert_chunk(chunk_offset, ChunkList::Available) {
            return None;
        }
        self.virgin_chunk_cursor = next_cursor;
        Some(chunk_offset)
    }

    fn ensure_available(&mut self) -> bool {
        while self.available_head == NO_LINK {
            if self.activate_next_chunk().is_none() {
                return false;
            }
        }
        true
    }

    fn allocate_frame(&mut self) -> Option<Frame> {
        if !self.ensure_available() {
            return None;
        }
        let chunk_offset = self.available_head;
        let addresses = self.addresses;
        let range_base = self.base;
        let (frame, list) = self.with_chunk_mut(chunk_offset, |chunk| {
            let recycled = !chunk.free_stack.is_empty();
            let page_index = if recycled {
                chunk.free_stack.peek()?
            } else {
                if chunk.virgin_cursor >= chunk.page_count {
                    return None;
                }
                chunk.virgin_cursor
            };
            if chunk.allocated.get(usize::from(page_index))? {
                return None;
            }
            let data_offset = chunk_offset.checked_add(PAGE_SIZE)?;
            let page_offset = usize::from(page_index).checked_mul(PAGE_SIZE)?;
            let address = addresses.translate(data_offset.checked_add(page_offset)?)?;
            if !chunk.allocated.set(usize::from(page_index), true) {
                return None;
            }
            if recycled {
                if chunk.free_stack.pop() != Some(page_index) {
                    let _restored = chunk.allocated.set(usize::from(page_index), false);
                    return None;
                }
            } else {
                chunk.virgin_cursor += 1;
            }
            let list = if chunk.free_pages() == 0 {
                ChunkList::Full
            } else {
                ChunkList::Available
            };
            Some((
                Frame {
                    address: address.as_usize(),
                    range_base,
                    chunk_offset,
                    page_index,
                },
                list,
            ))
        })??;
        if !self.move_chunk(chunk_offset, list) {
            return None;
        }
        Some(frame)
    }

    #[allow(dead_code)]
    fn allocate_contiguous(&mut self, page_count: u16, alignment: usize) -> Option<FrameRun> {
        if page_count == 0 || alignment < PAGE_SIZE || !alignment.is_power_of_two() {
            return None;
        }

        let mut offset = self.available_head;
        while offset != NO_LINK {
            let next = self.with_chunk_mut(offset, |chunk| chunk.links.next())?;
            if let Some(run) = self.allocate_from_virgin(offset, page_count, alignment) {
                return Some(run);
            }
            offset = next;
        }

        while self.virgin_chunk_cursor < self.length {
            let Some(offset) = self.activate_next_chunk() else {
                break;
            };
            if let Some(run) = self.allocate_from_virgin(offset, page_count, alignment) {
                return Some(run);
            }
        }

        offset = self.available_head;
        while offset != NO_LINK {
            let next = self.with_chunk_mut(offset, |chunk| chunk.links.next())?;
            if let Some(run) = self.allocate_from_recycled(offset, page_count, alignment) {
                return Some(run);
            }
            offset = next;
        }
        None
    }

    #[allow(dead_code)]
    fn allocate_from_virgin(
        &mut self,
        chunk_offset: usize,
        page_count: u16,
        alignment: usize,
    ) -> Option<FrameRun> {
        let addresses = self.addresses;
        let range_base = self.base;
        let (run, list) = self.with_chunk_mut(chunk_offset, |chunk| {
            let cursor = usize::from(chunk.virgin_cursor);
            let total = usize::from(chunk.page_count);
            let requested = usize::from(page_count);
            if cursor >= total || requested > total - cursor {
                return None;
            }
            let data_offset = chunk_offset.checked_add(PAGE_SIZE)?;
            let cursor_address = addresses.translate(data_offset.checked_add(cursor * PAGE_SIZE)?)?;
            let aligned_address = Self::align_up(cursor_address.as_usize(), alignment)?;
            let aligned_offset = addresses.offset_of(PhysAddr::new(aligned_address))?;
            let page_offset = aligned_offset.checked_sub(data_offset)?;
            if page_offset % PAGE_SIZE != 0 {
                return None;
            }
            let start = page_offset / PAGE_SIZE;
            let end = start.checked_add(requested)?;
            if start < cursor || end > total {
                return None;
            }
            let gap = start - cursor;
            if chunk.free_stack.len().checked_add(gap)? > chunk.free_stack.capacity() {
                return None;
            }
            let address = addresses.translate(data_offset.checked_add(start * PAGE_SIZE)?)?;
            for page in cursor..end {
                if chunk.allocated.get(page)? {
                    return None;
                }
            }

            let mut pushed = 0;
            for page in cursor..start {
                if chunk.free_stack.push(page as u16).is_err() {
                    for _ in 0..pushed {
                        let _discarded = chunk.free_stack.pop();
                    }
                    return None;
                }
                pushed += 1;
            }
            for page in start..end {
                if !chunk.allocated.set(page, true) {
                    for allocated_page in start..page {
                        let _restored = chunk.allocated.set(allocated_page, false);
                    }
                    for _ in 0..pushed {
                        let _discarded = chunk.free_stack.pop();
                    }
                    return None;
                }
            }
            chunk.virgin_cursor = end as u16;
            let list = if chunk.free_pages() == 0 {
                ChunkList::Full
            } else {
                ChunkList::Available
            };
            Some((
                FrameRun {
                    address,
                    range_base,
                    chunk_offset,
                    start_page: start as u16,
                    page_count,
                },
                list,
            ))
        })??;
        if !self.move_chunk(chunk_offset, list) {
            return None;
        }
        Some(run)
    }

    #[allow(dead_code)]
    fn allocate_from_recycled(
        &mut self,
        chunk_offset: usize,
        page_count: u16,
        alignment: usize,
    ) -> Option<FrameRun> {
        let addresses = self.addresses;
        let range_base = self.base;
        let (run, list) = self.with_chunk_mut(chunk_offset, |chunk| {
            let requested = usize::from(page_count);
            let cursor = usize::from(chunk.virgin_cursor);
            let mut free_run = 0usize;
            let mut selected = None;
            for end in 0..cursor {
                if chunk.allocated.get(end)? {
                    free_run = 0;
                    continue;
                }
                free_run += 1;
                if free_run < requested {
                    continue;
                }
                let end_exclusive = end + 1;
                let start = end_exclusive - requested;
                let data_offset = chunk_offset.checked_add(PAGE_SIZE)?;
                let address = addresses.translate(data_offset.checked_add(start * PAGE_SIZE)?)?;
                if address.as_usize() & (alignment - 1) == 0 {
                    selected = Some((start, end_exclusive, address));
                    break;
                }
            }
            let (start, end, address) = selected?;
            chunk.free_stack.retain(|page| {
                let page = usize::from(page);
                page < start || page >= end
            });
            let mut marked = 0usize;
            for page in start..end {
                if !chunk.allocated.set(page, true) {
                    for allocated_page in start..start + marked {
                        let _restored = chunk.allocated.set(allocated_page, false);
                    }
                    for page in start..end {
                        let _restored = chunk.free_stack.push(page as u16);
                    }
                    return None;
                }
                marked += 1;
            }
            let list = if chunk.free_pages() == 0 {
                ChunkList::Full
            } else {
                ChunkList::Available
            };
            Some((
                FrameRun {
                    address,
                    range_base,
                    chunk_offset,
                    start_page: start as u16,
                    page_count,
                },
                list,
            ))
        })??;
        if !self.move_chunk(chunk_offset, list) {
            return None;
        }
        Some(run)
    }

    fn deallocate_frame(&mut self, frame: Frame) -> bool {
        self.deallocate_run(FrameRun {
            address: PhysAddr::new(frame.address),
            range_base: frame.range_base,
            chunk_offset: frame.chunk_offset,
            start_page: frame.page_index,
            page_count: 1,
        })
    }

    fn run_for_address(&mut self, address: PhysAddr, page_count: u16) -> Option<FrameRun> {
        if page_count == 0 {
            return None;
        }
        let range_offset = self.addresses.offset_of(address)?;
        let relative = range_offset.checked_sub(PAGE_SIZE)?;
        let stride = (MAX_FRAMES_PER_CHUNK + 1).checked_mul(PAGE_SIZE)?;
        let chunk_number = relative / stride;
        let chunk_offset = PAGE_SIZE.checked_add(chunk_number.checked_mul(stride)?)?;
        if chunk_offset >= self.virgin_chunk_cursor {
            return None;
        }
        let data_page_offset = (relative % stride).checked_sub(PAGE_SIZE)?;
        if data_page_offset % PAGE_SIZE != 0 {
            return None;
        }
        let page_index = data_page_offset / PAGE_SIZE;
        let end_page = page_index.checked_add(usize::from(page_count))?;
        let chunk_page_count = self.with_chunk_mut(chunk_offset, |chunk| chunk.page_count)?;
        if end_page > usize::from(chunk_page_count) {
            return None;
        }
        Some(FrameRun {
            address,
            range_base: self.base,
            chunk_offset,
            start_page: u16::try_from(page_index).ok()?,
            page_count,
        })
    }

    fn deallocate_run(&mut self, run: FrameRun) -> bool {
        if run.range_base != self.base {
            return false;
        }
        let chunk_offset = run.chunk_offset;
        let addresses = self.addresses;
        let result = self.with_chunk_mut(chunk_offset, |chunk| {
            let start = usize::from(run.start_page);
            let count = usize::from(run.page_count);
            let Some(end) = start.checked_add(count) else {
                return None;
            };
            let data_offset = chunk_offset.checked_add(PAGE_SIZE)?;
            let expected_address = addresses.translate(data_offset.checked_add(start.checked_mul(PAGE_SIZE)?)?)?;
            if count == 0 || end > usize::from(chunk.page_count) || expected_address != run.address {
                return None;
            }
            let cursor = usize::from(chunk.virgin_cursor);
            if end > cursor {
                return None;
            }
            for page in start..end {
                if chunk.allocated.get(page) != Some(true) {
                    return None;
                }
            }
            let old_list = chunk.list;
            let rewind = chunk.free_stack.is_empty() && end == cursor;
            if !rewind && chunk.free_stack.len().checked_add(count)? > chunk.free_stack.capacity() {
                return None;
            }
            for page in start..end {
                if !chunk.allocated.set(page, false) {
                    for restored_page in start..page {
                        let _restored = chunk.allocated.set(restored_page, true);
                    }
                    return None;
                }
            }
            if rewind {
                chunk.virgin_cursor = start as u16;
            } else {
                let mut pushed = 0usize;
                for page in start..end {
                    if chunk.free_stack.push(page as u16).is_err() {
                        for _ in 0..pushed {
                            let _discarded = chunk.free_stack.pop();
                        }
                        for restored_page in start..end {
                            let _restored = chunk.allocated.set(restored_page, true);
                        }
                        return None;
                    }
                    pushed += 1;
                }
            }
            let new_list = if chunk.free_pages() == 0 {
                ChunkList::Full
            } else {
                ChunkList::Available
            };
            Some((old_list, new_list))
        });
        let Some(Some((old_list, new_list))) = result else {
            return false;
        };
        old_list == new_list || self.move_chunk(chunk_offset, new_list)
    }

    #[allow(dead_code)]
    fn align_up(address: usize, alignment: usize) -> Option<usize> {
        address
            .checked_add(alignment - 1)
            .map(|value| value & !(alignment - 1))
    }
}

struct PhysicalAllocatorState {
    range_head: usize,
    range_window: usize,
    chunk_window: usize,
    frame_count: usize,
    free_frames: usize,
    memory_bytes: usize,
    initialized: bool,
}

impl PhysicalAllocatorState {
    const fn new() -> Self {
        Self {
            range_head: NO_LINK,
            range_window: 0,
            chunk_window: 0,
            frame_count: 0,
            free_frames: 0,
            memory_bytes: 0,
            initialized: false,
        }
    }
}

pub(crate) struct PhysicalAllocator {
    state: IrqSpinLock<PhysicalAllocatorState>,
}

impl PhysicalAllocator {
    const fn new() -> Self {
        Self {
            state: IrqSpinLock::new(PhysicalAllocatorState::new()),
        }
    }

    fn get_instance() -> &'static Self {
        static ALLOCATOR: PhysicalAllocator = PhysicalAllocator::new();
        &ALLOCATOR
    }

    pub(crate) fn initialize() -> bool {
        let allocator = Self::get_instance();
        if allocator.state.lock().initialized {
            return true;
        }
        let Some(range_window) = crate::kernel::memory::r#virtual::reserve(
            2 * PAGE_SIZE,
            PAGE_SIZE,
            2,
        ) else {
            return false;
        };
        let Some(chunk_window) = range_window.checked_add(PAGE_SIZE) else {
            return false;
        };
        {
            let mut state = allocator.state.lock();
            state.range_window = range_window;
            state.chunk_window = chunk_window;
        }

        let image_end = core::ptr::addr_of!(__bss_end) as usize;
        let physical_limit = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::physical_address_limit() as usize;
        let memory_regions = crate::kernel::memory::memblock::MemBlock::memory_regions();
        for region in memory_regions.iter() {
            let Some(region_end) = region.start.checked_add(region.length) else {
                continue;
            };
            let end = region_end.min(physical_limit) & !(PAGE_SIZE - 1);
            let Some(mut cursor) = align_up(region.start.max(image_end), PAGE_SIZE) else {
                continue;
            };
            while cursor < end {
                let mut covered_end = cursor;
                let mut next_reserved_start = end;
                crate::kernel::memory::memblock::MemBlock::for_each_reserved_range(
                    |reserved_start, reserved_length| {
                        let Some(reserved_end) = reserved_start.checked_add(reserved_length) else {
                            covered_end = end;
                            return;
                        };
                        let reserved_start = reserved_start & !(PAGE_SIZE - 1);
                        let Some(reserved_end) = align_up(reserved_end, PAGE_SIZE) else {
                            covered_end = end;
                            return;
                        };
                        if reserved_end <= cursor || reserved_start >= end {
                            return;
                        }
                        if reserved_start <= cursor {
                            covered_end = covered_end.max(reserved_end.min(end));
                        } else {
                            next_reserved_start = next_reserved_start.min(reserved_start);
                        }
                    },
                );
                if covered_end > cursor {
                    cursor = covered_end;
                    continue;
                }
                let range_end = next_reserved_start.min(end);
                if range_end > cursor && range_end - cursor >= 3 * PAGE_SIZE {
                    // SAFETY: this span is usable RAM, lies above the kernel image,
                    // excludes every memblock reservation, and scratch mappings are reserved.
                    if !unsafe {
                        allocator.add_memory_range(
                            PhysAddr::new(cursor),
                            range_end - cursor,
                        )
                    } {
                        return false;
                    }
                }
                cursor = range_end;
            }
        }

        let mut state = allocator.state.lock();
        state.initialized = state.frame_count != 0;
        state.initialized
    }

    unsafe fn add_memory_range(&self, base: PhysAddr, length: usize) -> bool {
        // SAFETY: the caller guarantees this is mapped usable RAM and its first
        // page is reserved for the in-range PhysicalMemory header.
        unsafe { self.state.lock().add_memory_range(base, length) }
    }

    pub(crate) fn allocate() -> Option<Frame> {
        Self::get_instance().state.lock().allocate()
    }

    #[allow(dead_code)]
    pub(crate) fn allocate_contiguous(length: usize, alignment: usize) -> Option<usize> {
        if length == 0 || length % PAGE_SIZE != 0 {
            return None;
        }
        let pages = u16::try_from(length / PAGE_SIZE).ok()?;
        Self::get_instance()
            .state
            .lock()
            .allocate_contiguous(pages, alignment)
            .map(|run| run.address.as_usize())
    }

    pub(crate) fn deallocate(frame: Frame) -> bool {
        Self::get_instance().state.lock().deallocate(frame)
    }

    pub(crate) fn deallocate_contiguous(address: usize, length: usize) -> bool {
        if length == 0 || length % PAGE_SIZE != 0 || address % PAGE_SIZE != 0 {
            return false;
        }
        let Ok(page_count) = u16::try_from(length / PAGE_SIZE) else {
            return false;
        };
        Self::get_instance()
            .state
            .lock()
            .deallocate_contiguous(PhysAddr::new(address), page_count)
    }

    #[allow(dead_code)]
    pub(crate) fn free_frames() -> usize {
        Self::get_instance().state.lock().free_frames
    }

    pub(crate) fn memory_bytes() -> usize {
        Self::get_instance().state.lock().memory_bytes
    }
}

impl PhysicalAllocatorState {
    unsafe fn add_memory_range(&mut self, base: PhysAddr, length: usize) -> bool {
        let Some(memory) = PhysicalMemory::new(base, length, self.chunk_window) else {
            return false;
        };
        if self.range_overlaps(base, length) != Some(false) {
            return false;
        }
        let Some(frame_count) = memory.usable_frame_count() else {
            return false;
        };
        let Some(memory_bytes) = frame_count.checked_mul(PAGE_SIZE) else {
            return false;
        };
        let Some(new_frame_count) = self.frame_count.checked_add(frame_count) else {
            return false;
        };
        let Some(new_free_frames) = self.free_frames.checked_add(frame_count) else {
            return false;
        };
        let Some(new_memory_bytes) = self.memory_bytes.checked_add(memory_bytes) else {
            return false;
        };

        let range_address = base.as_usize();
        let Some(()) = with_mapped_page(self.range_window, base, |mapped_address| {
            // SAFETY: add_memory_range reserves and maps this physical header page.
            unsafe { (mapped_address as *mut PhysicalMemory).write(memory) };
        }) else {
            return false;
        };
        if !self.insert_range(range_address) {
            return false;
        }
        self.frame_count = new_frame_count;
        self.free_frames = new_free_frames;
        self.memory_bytes = new_memory_bytes;
        true
    }

    fn range_overlaps(&mut self, base: PhysAddr, length: usize) -> Option<bool> {
        let end = base.checked_add(length)?;
        let head = self.range_head;
        if head == NO_LINK {
            return Some(false);
        }
        let mut current = head;
        loop {
            let (other_base, other_length, next) = self.with_range_mut(current, |range| {
                (range.base, range.length, range.range_links.next())
            })?;
            let other_end = other_base.checked_add(other_length)?;
            if base < other_end && other_base < end {
                return Some(true);
            }
            current = next;
            if current == head {
                return Some(false);
            }
        }
    }

    fn insert_range(&mut self, range_address: usize) -> bool {
        if self.range_head == NO_LINK {
            if self
                .with_range_mut(range_address, |range| {
                    range.range_links.set_previous(range_address);
                    range.range_links.set_next(range_address);
                })
                .is_none()
            {
                return false;
            }
            self.range_head = range_address;
            return true;
        }

        let head = self.range_head;
        let Some(tail) = self.with_range_mut(head, |range| range.range_links.previous()) else {
            return false;
        };
        if self
            .with_range_mut(range_address, |range| {
                range.range_links.set_previous(tail);
                range.range_links.set_next(head);
            })
            .is_none()
            || self
                .with_range_mut(tail, |range| range.range_links.set_next(range_address))
                .is_none()
            || self
                .with_range_mut(head, |range| range.range_links.set_previous(range_address))
                .is_none()
        {
            return false;
        }
        true
    }

    fn with_range_mut<R>(
        &mut self,
        range_address: usize,
        operation: impl for<'a> FnOnce(&'a mut PhysicalMemory) -> R,
    ) -> Option<R> {
        if range_address == NO_LINK || range_address % PAGE_SIZE != 0 {
            return None;
        }
        with_mapped_page(
            self.range_window,
            PhysAddr::new(range_address),
            |mapped_address| {
                // SAFETY: only registered range headers are passed here while the
                // allocator lock is held, and the closure cannot return a borrow.
                unsafe { operation(&mut *(mapped_address as *mut PhysicalMemory)) }
            },
        )
    }

    fn allocate(&mut self) -> Option<Frame> {
        if !self.initialized || self.free_frames == 0 {
            return None;
        }
        let head = self.range_head;
        if head == NO_LINK {
            return None;
        }
        let mut current = head;
        loop {
            let next = self.with_range_mut(current, |range| range.range_links.next())?;
            if let Some(frame) = self.with_range_mut(current, PhysicalMemory::allocate_frame)? {
                self.free_frames -= 1;
                return Some(frame);
            }
            current = next;
            if current == head {
                return None;
            }
        }
    }

    #[allow(dead_code)]
    fn allocate_contiguous(&mut self, page_count: u16, alignment: usize) -> Option<FrameRun> {
        if !self.initialized || usize::from(page_count) > self.free_frames {
            return None;
        }
        let head = self.range_head;
        if head == NO_LINK {
            return None;
        }
        let mut current = head;
        loop {
            let next = self.with_range_mut(current, |range| range.range_links.next())?;
            if let Some(run) = self.with_range_mut(current, |range| {
                range.allocate_contiguous(page_count, alignment)
            })? {
                self.free_frames -= usize::from(page_count);
                return Some(run);
            }
            current = next;
            if current == head {
                return None;
            }
        }
    }

    fn deallocate(&mut self, frame: Frame) -> bool {
        if !self.initialized {
            return false;
        }
        let Some(deallocated) = self
            .with_range_mut(frame.range_base.as_usize(), |range| range.deallocate_frame(frame))
        else {
            return false;
        };
        if deallocated {
            self.free_frames = self.free_frames.saturating_add(1);
        }
        deallocated
    }

    fn deallocate_contiguous(&mut self, address: PhysAddr, page_count: u16) -> bool {
        if !self.initialized || page_count == 0 {
            return false;
        }
        let head = self.range_head;
        if head == NO_LINK {
            return false;
        }
        let mut current = head;
        loop {
            let next = match self.with_range_mut(current, |range| range.range_links.next()) {
                Some(next) => next,
                None => return false,
            };
            let run = match self.with_range_mut(current, |range| {
                range.run_for_address(address, page_count)
            }) {
                Some(run) => run,
                None => return false,
            };
            if let Some(run) = run {
                let Some(deallocated) = self.with_range_mut(current, |range| range.deallocate_run(run)) else {
                    return false;
                };
                if deallocated {
                    self.free_frames = self.free_frames.saturating_add(usize::from(page_count));
                }
                return deallocated;
            }
            current = next;
            if current == head {
                return false;
            }
        }
    }
}

fn with_mapped_page<R>(
    virtual_address: usize,
    physical_address: PhysAddr,
    operation: impl FnOnce(usize) -> R,
) -> Option<R> {
    if virtual_address % PAGE_SIZE != 0
        || !MemoryArchitecture::map_page(virtual_address, physical_address.as_usize(), 2)
    {
        return None;
    }
    let result = operation(virtual_address);
    if MemoryArchitecture::unmap_page(virtual_address) != Some(physical_address.as_usize()) {
        return None;
    }
    Some(result)
}
