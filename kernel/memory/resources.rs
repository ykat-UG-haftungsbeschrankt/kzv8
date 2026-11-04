/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::mem::{align_of, size_of};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::kernel::scheduler::sync::IrqSpinLock;

const BOOTSTRAP_MEMORY_REGIONS: usize = 32;
const PAGE_SIZE: usize = 4096;

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
    regions: [MemoryRegion; BOOTSTRAP_MEMORY_REGIONS],
    dynamic_regions: usize,
    capacity: usize,
    count: usize,
    overflowed: bool,
}

impl MemoryRegions {
    const EMPTY: Self = Self {
        regions: [MemoryRegion::EMPTY; BOOTSTRAP_MEMORY_REGIONS],
        dynamic_regions: 0,
        capacity: BOOTSTRAP_MEMORY_REGIONS,
        count: 0,
        overflowed: false,
    };

    pub(crate) fn as_slice(&self) -> &[MemoryRegion] {
        if self.dynamic_regions == 0 {
            &self.regions[..self.count]
        } else {
            // SAFETY: dynamic storage is allocated for `capacity` initialized regions and is
            // retained for the lifetime of the registry.
            unsafe {
                core::slice::from_raw_parts(
                    self.dynamic_regions as *const MemoryRegion,
                    self.count,
                )
            }
        }
    }

    fn as_mut_capacity_slice(&mut self) -> &mut [MemoryRegion] {
        if self.dynamic_regions == 0 {
            &mut self.regions[..self.capacity]
        } else {
            // SAFETY: `grow` initializes all `capacity` entries before publishing this pointer.
            unsafe {
                core::slice::from_raw_parts_mut(
                    self.dynamic_regions as *mut MemoryRegion,
                    self.capacity,
                )
            }
        }
    }

    fn grow(&mut self) -> bool {
        let Some(capacity) = self.capacity.checked_mul(2) else {
            return false;
        };
        let Some(bytes) = capacity.checked_mul(size_of::<MemoryRegion>()) else {
            return false;
        };
        let Some(address) = allocate_boot_memory(bytes, align_of::<MemoryRegion>()) else {
            return false;
        };
        let destination = address as *mut MemoryRegion;
        // SAFETY: the boot allocator returns aligned writable storage of `bytes` bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(self.as_slice().as_ptr(), destination, self.count);
            for index in self.count..capacity {
                destination.add(index).write(MemoryRegion::EMPTY);
            }
        }
        self.dynamic_regions = address;
        self.capacity = capacity;
        true
    }

    pub(crate) fn iter(&self) -> core::slice::Iter<'_, MemoryRegion> {
        self.as_slice().iter()
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.count == 0
    }
}

pub(crate) struct MemoryRegionRegistry {
    regions: IrqSpinLock<MemoryRegions>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EarlyAllocation {
    next: usize,
    start: usize,
    length: usize,
}

static DYNAMIC_STORAGE_ENABLED: AtomicBool = AtomicBool::new(false);
static BOOTSTRAP_REGIONS: IrqSpinLock<MemoryRegions> = IrqSpinLock::new(MemoryRegions::EMPTY);
static EARLY_ALLOCATIONS: IrqSpinLock<usize> = IrqSpinLock::new(0);

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
}

fn allocate_boot_memory(length: usize, alignment: usize) -> Option<usize> {
    if length == 0 || alignment == 0 || !alignment.is_power_of_two() {
        return None;
    }
    let data_offset = align_up(size_of::<EarlyAllocation>(), alignment)?;
    let total_length = data_offset.checked_add(length)?;
    let allocation_length = total_length
        .checked_add(PAGE_SIZE - 1)?
        & !(PAGE_SIZE - 1);
    let identity_limit =
        <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::identity_mapped_limit()
            as usize;
    let image_end = core::ptr::addr_of!(__bss_end) as usize;
    let bootstrap_regions = *BOOTSTRAP_REGIONS.lock();
    let mut allocations = EARLY_ALLOCATIONS.lock();

    for region in bootstrap_regions.iter() {
        let Some(region_end) = region.start.checked_add(region.length) else {
            continue;
        };
        let region_start = region.start.max(image_end);
        let region_end = region_end.min(identity_limit) & !(PAGE_SIZE - 1);
        let Some(region_start) = align_up(region_start, PAGE_SIZE) else {
            continue;
        };
        let mut candidate_end = region_end;
        loop {
            let Some(candidate_start) = candidate_end.checked_sub(allocation_length) else {
                break;
            };
            if candidate_start < region_start {
                break;
            }
            let candidate_end = candidate_start + allocation_length;
            let mut overlap_start = None;
            let mut current = *allocations;
            while current != 0 {
                // SAFETY: allocation headers are stored in identity-mapped RAM and kept allocated.
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
            if let Some(overlap_start) = overlap_start {
                candidate_end = overlap_start;
                continue;
            }

            let allocation = EarlyAllocation {
                next: *allocations,
                start: candidate_start,
                length: allocation_length,
            };
            // SAFETY: the candidate range is page-aligned usable RAM, identity-mapped, and disjoint
            // from the kernel image and all prior bootstrap allocations.
            unsafe { (candidate_start as *mut EarlyAllocation).write(allocation) };
            *allocations = candidate_start;
            return candidate_start.checked_add(data_offset);
        }
    }
    None
}

impl MemoryRegionRegistry {
    const fn new() -> Self {
        Self {
            regions: IrqSpinLock::new(MemoryRegions::EMPTY),
        }
    }

    fn get_instance() -> &'static Self {
        static REGISTRY: MemoryRegionRegistry = MemoryRegionRegistry::new();
        &REGISTRY
    }

    pub(crate) fn reset_memory_regions() {
        *Self::get_instance().regions.lock() = MemoryRegions::EMPTY;
    }

    pub(crate) fn needs_growth() -> bool {
        Self::get_instance().regions.lock().overflowed
    }

    pub(crate) fn enable_dynamic_storage() {
        let bootstrap_regions = *Self::get_instance().regions.lock();
        *BOOTSTRAP_REGIONS.lock() = bootstrap_regions;
        DYNAMIC_STORAGE_ENABLED.store(true, Ordering::Release);
    }

    pub(crate) fn allocate_boot_memory(length: usize, alignment: usize) -> Option<usize> {
        allocate_boot_memory(length, alignment)
    }

    pub(crate) fn for_each_boot_allocation(mut visit: impl FnMut(usize, usize)) {
        let allocations = EARLY_ALLOCATIONS.lock();
        let mut current = *allocations;
        while current != 0 {
            // SAFETY: allocation headers remain mapped and allocated until after physical init.
            let allocation = unsafe { (current as *const EarlyAllocation).read() };
            visit(allocation.start, allocation.length);
            current = allocation.next;
        }
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
        let clipped_end = end.min(maximum);
        if clipped_end <= start {
            return true;
        }
        let mut start = start as usize;
        let mut end = clipped_end as usize;
        let mut regions = Self::get_instance().regions.lock();
        let mut index = 0;
        while index < regions.count {
            let current = regions.as_slice()[index];
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
            {
                let storage = regions.as_mut_capacity_slice();
                for slot in index..regions.count - 1 {
                    storage[slot] = storage[slot + 1];
                }
            }
            regions.count -= 1;
        }
        if regions.count == regions.capacity {
            if !DYNAMIC_STORAGE_ENABLED.load(Ordering::Acquire) {
                regions.overflowed = true;
                return true;
            }
            if !regions.grow() {
                return false;
            }
        }
        {
            let storage = regions.as_mut_capacity_slice();
            for slot in (index..regions.count).rev() {
                storage[slot + 1] = storage[slot];
            }
            storage[index] = MemoryRegion {
                start,
                length: end - start,
            };
        }
        regions.count += 1;
        true
    }

    pub(crate) fn memory_regions() -> MemoryRegions {
        *Self::get_instance().regions.lock()
    }
}
