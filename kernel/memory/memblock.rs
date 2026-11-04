use core::mem::{align_of, size_of};

use crate::kernel::scheduler::sync::IrqSpinLock;

const INITIAL_REGION_CAPACITY: usize = 32;
const PAGE_SIZE: usize = 4096;

extern "C" {
    static __bss_end: u8;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MemoryRegion {
    pub(crate) start: usize,
    pub(crate) length: usize,
}

impl MemoryRegion {
    const EMPTY: Self = Self {
        start: 0,
        length: 0,
    };
}

#[derive(Clone, Copy)]
pub(crate) struct MemoryRegions {
    entries: [MemoryRegion; INITIAL_REGION_CAPACITY],
    dynamic_entries: usize,
    capacity: usize,
    count: usize,
}

impl MemoryRegions {
    const EMPTY: Self = Self {
        entries: [MemoryRegion::EMPTY; INITIAL_REGION_CAPACITY],
        dynamic_entries: 0,
        capacity: INITIAL_REGION_CAPACITY,
        count: 0,
    };

    pub(crate) fn as_slice(&self) -> &[MemoryRegion] {
        if self.dynamic_entries == 0 {
            &self.entries[..self.count]
        } else {
            // SAFETY: the memblock keeps its dynamically allocated entries alive permanently.
            unsafe {
                core::slice::from_raw_parts(self.dynamic_entries as *const MemoryRegion, self.count)
            }
        }
    }

    fn as_mut_capacity_slice(&mut self) -> &mut [MemoryRegion] {
        if self.dynamic_entries == 0 {
            &mut self.entries[..self.capacity]
        } else {
            // SAFETY: growth initializes all entries before publishing this pointer.
            unsafe {
                core::slice::from_raw_parts_mut(
                    self.dynamic_entries as *mut MemoryRegion,
                    self.capacity,
                )
            }
        }
    }

    fn insert(&mut self, mut start: usize, mut end: usize) -> bool {
        if start >= end {
            return false;
        }
        let mut index = 0;
        while index < self.count {
            let current = self.as_slice()[index];
            let Some(current_end) = current.start.checked_add(current.length) else {
                return false;
            };
            if current_end < start {
                index += 1;
                continue;
            }
            if end < current.start {
                break;
            }
            start = start.min(current.start);
            end = end.max(current_end);
            let count = self.count;
            let entries = self.as_mut_capacity_slice();
            for slot in index..count - 1 {
                entries[slot] = entries[slot + 1];
            }
            self.count -= 1;
        }
        if self.count == self.capacity {
            return false;
        }
        let count = self.count;
        let entries = self.as_mut_capacity_slice();
        for slot in (index..count).rev() {
            entries[slot + 1] = entries[slot];
        }
        entries[index] = MemoryRegion {
            start,
            length: end - start,
        };
        self.count += 1;
        true
    }

    pub(crate) fn iter(&self) -> core::slice::Iter<'_, MemoryRegion> {
        self.as_slice().iter()
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EarlyAllocation {
    next: usize,
    start: usize,
    length: usize,
}

struct MemBlockState {
    memory: MemoryRegions,
    bootstrap_memory: MemoryRegions,
    reserved: MemoryRegions,
    bootstrap_candidate: MemoryRegion,
    early_allocations: usize,
    memory_map_overflowed: bool,
    growth_enabled: bool,
    discovery_only: bool,
    saw_memory_region: bool,
}

impl MemBlockState {
    const EMPTY: Self = Self {
        memory: MemoryRegions::EMPTY,
        bootstrap_memory: MemoryRegions::EMPTY,
        reserved: MemoryRegions::EMPTY,
        bootstrap_candidate: MemoryRegion::EMPTY,
        early_allocations: 0,
        memory_map_overflowed: false,
        growth_enabled: false,
        discovery_only: false,
        saw_memory_region: false,
    };

    fn grow_regions(&mut self, regions: MemoryRegions) -> Option<MemoryRegions> {
        let capacity = regions.capacity.checked_mul(2)?;
        let bytes = capacity.checked_mul(size_of::<MemoryRegion>())?;
        let address = self.allocate_early_storage(bytes, align_of::<MemoryRegion>())?;
        let destination = address as *mut MemoryRegion;
        let source = regions.as_slice().as_ptr();
        // SAFETY: the boot allocator returns disjoint storage for `capacity` entries.
        unsafe {
            core::ptr::copy_nonoverlapping(source, destination, regions.count);
            for index in regions.count..capacity {
                destination.add(index).write(MemoryRegion::EMPTY);
            }
        }
        Some(MemoryRegions {
            entries: [MemoryRegion::EMPTY; INITIAL_REGION_CAPACITY],
            dynamic_entries: address,
            capacity,
            count: regions.count,
        })
    }

    fn allocate_early_storage(&mut self, length: usize, alignment: usize) -> Option<usize> {
        if !self.growth_enabled
            || length == 0
            || alignment == 0
            || !alignment.is_power_of_two()
        {
            return None;
        }
        let allocation_alignment = alignment.max(PAGE_SIZE);
        let data_offset = align_up(size_of::<EarlyAllocation>(), alignment)?;
        let identity_limit = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::identity_mapped_limit()
            as usize;
        let bootstrap_start = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::bootstrap_memory_start()
            as usize;
        let image_end = core::ptr::addr_of!(__bss_end) as usize;
        let bootstrap_memory = self.bootstrap_memory;
        let memory = self.memory;
        let candidate = self.bootstrap_candidate;
        let reserved = self.reserved;
        let mut allocations = self.early_allocations;

        for region in bootstrap_memory.iter() {
            if let Some(address) = allocate_from_region(
                *region,
                length,
                allocation_alignment,
                data_offset,
                image_end,
                bootstrap_start,
                identity_limit,
                &reserved,
                &mut allocations,
            ) {
                self.early_allocations = allocations;
                return Some(address);
            }
        }
        for region in memory.iter() {
            if !<crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::map_boot_memory_region(
                region.start,
                region.length,
            ) {
                continue;
            }
            if let Some(address) = allocate_from_region(
                *region,
                length,
                allocation_alignment,
                data_offset,
                image_end,
                bootstrap_start,
                identity_limit,
                &reserved,
                &mut allocations,
            ) {
                self.early_allocations = allocations;
                return Some(address);
            }
        }
        if let Some(address) = allocate_from_region(
            candidate,
            length,
            allocation_alignment,
            data_offset,
            image_end,
            bootstrap_start,
            identity_limit,
            &reserved,
            &mut allocations,
        ) {
            self.early_allocations = allocations;
            return Some(address);
        }
        None
    }

    fn allocatable_subregion(&self, region: MemoryRegion) -> MemoryRegion {
        let identity_limit = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::identity_mapped_limit()
            as usize;
        let bootstrap_start = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::bootstrap_memory_start()
            as usize;
        let image_end = core::ptr::addr_of!(__bss_end) as usize;
        let Some(region_end) = region.start.checked_add(region.length) else {
            return MemoryRegion::EMPTY;
        };
        let region_end = region_end.min(identity_limit) & !(PAGE_SIZE - 1);
        let Some(mut cursor) = align_up(
            region.start.max(image_end).max(bootstrap_start),
            PAGE_SIZE,
        ) else {
            return MemoryRegion::EMPTY;
        };
        if cursor >= region_end {
            return MemoryRegion::EMPTY;
        }

        let mut largest = MemoryRegion::EMPTY;
        for reserved in self.reserved.iter() {
            let Some(reserved_end) = reserved.start.checked_add(reserved.length) else {
                continue;
            };
            let reserved_start = reserved.start & !(PAGE_SIZE - 1);
            let reserved_end = align_up(reserved_end, PAGE_SIZE).unwrap_or(usize::MAX);
            if reserved_end <= cursor || reserved_start >= region_end {
                continue;
            }
            let free_end = reserved_start.min(region_end);
            if free_end > cursor && free_end - cursor > largest.length {
                largest = MemoryRegion {
                    start: cursor,
                    length: free_end - cursor,
                };
            }
            cursor = cursor.max(reserved_end.min(region_end));
            if cursor >= region_end {
                break;
            }
        }
        if region_end > cursor && region_end - cursor > largest.length {
            largest = MemoryRegion {
                start: cursor,
                length: region_end - cursor,
            };
        }
        largest
    }
}

pub(crate) struct MemBlock;

impl MemBlock {
    fn get_instance() -> &'static IrqSpinLock<MemBlockState> {
        static MEMBLOCK: IrqSpinLock<MemBlockState> = IrqSpinLock::new(MemBlockState::EMPTY);
        &MEMBLOCK
    }

    pub(crate) fn reset() {
        let mut state = Self::get_instance().lock();
        state.memory = MemoryRegions::EMPTY;
        state.reserved = MemoryRegions::EMPTY;
        state.memory_map_overflowed = false;
        state.saw_memory_region = false;
    }

    pub(crate) fn begin_discovery() {
        let mut state = Self::get_instance().lock();
        state.discovery_only = true;
        state.bootstrap_candidate = MemoryRegion::EMPTY;
    }

    pub(crate) fn finish_discovery() -> bool {
        let mut state = Self::get_instance().lock();
        state.discovery_only = false;
        let candidate = state.bootstrap_candidate;
        candidate.length == 0
            || <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::map_boot_memory_region(
                candidate.start,
                candidate.length,
            )
    }

    pub(crate) fn reserve_region(start: usize, length: usize) -> bool {
        if length == 0 {
            return true;
        }
        let Some(end) = start.checked_add(length) else {
            return false;
        };
        let mut state = Self::get_instance().lock();
        if state.reserved.insert(start, end) {
            return true;
        }
        if !state.growth_enabled {
            return false;
        }
        let reserved = state.reserved;
        let Some(reserved) = state.grow_regions(reserved) else {
            return false;
        };
        state.reserved = reserved;
        state.reserved.insert(start, end)
    }

    pub(crate) fn needs_growth() -> bool {
        Self::get_instance().lock().memory_map_overflowed
    }

    pub(crate) fn has_memory_regions() -> bool {
        Self::get_instance().lock().saw_memory_region
    }

    pub(crate) fn enable_growth() {
        let mut state = Self::get_instance().lock();
        state.bootstrap_memory = state.memory;
        state.growth_enabled = true;
    }

    pub(crate) fn add_memory_region(start: u64, length: u64) -> bool {
        if length == 0 {
            return true;
        }
        let Some(end) = start.checked_add(length) else {
            return false;
        };
        let maximum = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::physical_address_limit();
        if start >= maximum {
            return true;
        }
        let end = end.min(maximum);
        if end <= start {
            return true;
        }
        let start = start as usize;
        let end = end as usize;
        let mut state = Self::get_instance().lock();
        state.saw_memory_region = true;
        if state.growth_enabled && state.discovery_only {
            let candidate = state.allocatable_subregion(MemoryRegion {
                start,
                length: end - start,
            });
            if candidate.length > state.bootstrap_candidate.length {
                state.bootstrap_candidate = candidate;
            }
            if state.discovery_only {
                return true;
            }
        }
        if state.memory.insert(start, end) {
            return true;
        }
        if !state.growth_enabled {
            state.memory_map_overflowed = true;
            return true;
        }
        let memory = state.memory;
        let Some(memory) = state.grow_regions(memory) else {
            return false;
        };
        state.memory = memory;
        state.memory.insert(start, end)
    }

    pub(crate) fn allocate_early(length: usize, alignment: usize) -> Option<usize> {
        Self::get_instance()
            .lock()
            .allocate_early_storage(length, alignment)
    }

    pub(crate) fn for_each_reserved_range(mut visit: impl FnMut(usize, usize)) {
        let (reserved, mut current) = {
            let state = Self::get_instance().lock();
            (state.reserved, state.early_allocations)
        };
        for region in reserved.iter() {
            visit(region.start, region.length);
        }
        while current != 0 {
            // SAFETY: allocation headers remain mapped and allocated until physical init.
            let allocation = unsafe { (current as *const EarlyAllocation).read() };
            visit(allocation.start, allocation.length);
            current = allocation.next;
        }
    }

    pub(crate) fn memory_regions() -> MemoryRegions {
        Self::get_instance().lock().memory
    }
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
}

fn allocate_from_region(
    region: MemoryRegion,
    length: usize,
    allocation_alignment: usize,
    data_offset: usize,
    image_end: usize,
    bootstrap_start: usize,
    identity_limit: usize,
    reserved: &MemoryRegions,
    allocations: &mut usize,
) -> Option<usize> {
    let region_end = region.start.checked_add(region.length)?.min(identity_limit) & !(PAGE_SIZE - 1);
    let region_start = align_up(region.start.max(image_end).max(bootstrap_start), PAGE_SIZE)?;
    if region_start >= region_end {
        return None;
    }

    let allocation_length = align_up(data_offset.checked_add(length)?, allocation_alignment)?;
    let mut candidate_end = region_end;
    loop {
        let raw_start = candidate_end.checked_sub(allocation_length)?;
        let candidate_start = raw_start & !(allocation_alignment - 1);
        if candidate_start < region_start {
            return None;
        }
        let candidate_end = candidate_start.checked_add(allocation_length)?;
        let mut overlap_start = None;

        let mut current = *allocations;
        while current != 0 {
            // SAFETY: allocation headers are in mapped RAM and are never freed or overwritten.
            let allocation = unsafe { (current as *const EarlyAllocation).read() };
            let Some(allocation_end) = allocation.start.checked_add(allocation.length) else {
                return None;
            };
            if candidate_start < allocation_end && allocation.start < candidate_end {
                overlap_start = Some(allocation.start);
                break;
            }
            current = allocation.next;
        }
        for range in reserved.iter() {
            let Some(reserved_end) = range.start.checked_add(range.length) else {
                continue;
            };
            if candidate_start < reserved_end && range.start < candidate_end {
                overlap_start = Some(overlap_start.map_or(range.start, |start: usize| {
                    start.min(range.start)
                }));
            }
        }
        if let Some(overlap_start) = overlap_start {
            candidate_end = overlap_start & !(allocation_alignment - 1);
            continue;
        }

        let allocation = EarlyAllocation {
            next: *allocations,
            start: candidate_start,
            length: allocation_length,
        };
        // SAFETY: this page-aligned, identity-mapped usable range is disjoint from all reservations.
        unsafe { (candidate_start as *mut EarlyAllocation).write(allocation) };
        *allocations = candidate_start;
        return candidate_start.checked_add(data_offset);
    }
}