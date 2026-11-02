/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::scheduler::sync::IrqSpinLock;

pub(crate) const MAX_PHYSICAL_ADDRESS: usize = 64 * 1024 * 1024 * 1024;
const MAX_MEMORY_REGIONS: usize = 32;

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
    regions: [MemoryRegion; MAX_MEMORY_REGIONS],
    count: usize,
}

impl MemoryRegions {
    const EMPTY: Self = Self {
        regions: [MemoryRegion::EMPTY; MAX_MEMORY_REGIONS],
        count: 0,
    };

    pub(crate) fn as_slice(&self) -> &[MemoryRegion] {
        &self.regions[..self.count]
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

    pub(crate) fn add_memory_region(start: u64, length: u64) -> bool {
        if length == 0 {
            return true;
        }
        let Some(end) = start.checked_add(length) else {
            return false;
        };
        let maximum = MAX_PHYSICAL_ADDRESS as u64;
        if start >= maximum {
            return true;
        }
        let clipped_end = end.min(maximum);
        let region = MemoryRegion {
            start: start as usize,
            length: (clipped_end - start) as usize,
        };
        let mut regions = Self::get_instance().regions.lock();
        if regions.count == MAX_MEMORY_REGIONS {
            return false;
        }
        let index = regions.count;
        regions.regions[index] = region;
        regions.count = index + 1;
        true
    }

    pub(crate) fn memory_regions() -> MemoryRegions {
        *Self::get_instance().regions.lock()
    }
}
