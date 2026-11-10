/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::lib::endian::{read_u32, read_u64};

/// # Safety
/// `boot_info` must point to a readable Multiboot info structure and memory map.
pub(crate) unsafe fn initialize(boot_info: usize, boot_magic: usize) -> bool {
    const MULTIBOOT_MAGIC: usize = 0x2bad_b002;
    const MEMORY_MAP_FLAG: u32 = 1 << 6;
    if boot_magic != MULTIBOOT_MAGIC || boot_info == 0 {
        return false;
    }

    let boot_info_address = boot_info;
    let boot_info = unsafe { core::slice::from_raw_parts(boot_info as *const u8, 52) };
    let Some(flags) = read_u32(boot_info, 0) else {
        return false;
    };
    if flags & MEMORY_MAP_FLAG == 0 {
        return false;
    }
    let Some(map_length) = read_u32(boot_info, 44).map(|value| value as usize) else {
        return false;
    };
    let Some(map_start) = read_u32(boot_info, 48).map(|value| value as usize) else {
        return false;
    };
    if map_start == 0 || map_length == 0 {
        return false;
    }
    let map = unsafe { core::slice::from_raw_parts(map_start as *const u8, map_length) };

    crate::kernel::memory::memblock::MemBlock::reset();
    if !crate::kernel::memory::memblock::MemBlock::reserve_region(boot_info_address, 52)
        || !crate::kernel::memory::memblock::MemBlock::reserve_region(map_start, map_length)
    {
        return false;
    }
    let mut entry = 0;
    while entry < map.len() {
        let remaining = map.len() - entry;
        if remaining < 4 {
            return false;
        }
        let Some(size) = read_u32(map, entry).map(|value| value as usize) else {
            return false;
        };
        let Some(entry_length) = size.checked_add(4) else {
            return false;
        };
        if size < 20 || entry_length > remaining {
            return false;
        }
        let (Some(start), Some(length), Some(kind)) = (
            read_u64(map, entry + 4),
            read_u64(map, entry + 12),
            read_u32(map, entry + 20),
        ) else {
            return false;
        };
        if kind == 1
            && !crate::kernel::memory::memblock::MemBlock::add_memory_region(start, length)
        {
            return false;
        }
        entry += entry_length;
    }
    crate::kernel::memory::memblock::MemBlock::has_memory_regions()
}
