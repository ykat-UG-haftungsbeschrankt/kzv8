/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicUsize, Ordering};

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

static mut MEMORY_REGIONS: [MemoryRegion; MAX_MEMORY_REGIONS] =
    [MemoryRegion::EMPTY; MAX_MEMORY_REGIONS];
static MEMORY_REGION_COUNT: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn reset_memory_regions() {
    unsafe {
        let regions = addr_of_mut!(MEMORY_REGIONS).cast::<MemoryRegion>();
        for index in 0..MAX_MEMORY_REGIONS {
            regions.add(index).write(MemoryRegion::EMPTY);
        }
    }
    MEMORY_REGION_COUNT.store(0, Ordering::Release);
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
    let count = MEMORY_REGION_COUNT.load(Ordering::Relaxed);
    if count == MAX_MEMORY_REGIONS {
        return false;
    }
    unsafe {
        addr_of_mut!(MEMORY_REGIONS)
            .cast::<MemoryRegion>()
            .add(count)
            .write(region);
    }
    MEMORY_REGION_COUNT.store(count + 1, Ordering::Release);
    true
}

pub(crate) fn memory_regions() -> &'static [MemoryRegion] {
    let count = MEMORY_REGION_COUNT.load(Ordering::Acquire);
    unsafe { core::slice::from_raw_parts(addr_of!(MEMORY_REGIONS).cast::<MemoryRegion>(), count) }
}