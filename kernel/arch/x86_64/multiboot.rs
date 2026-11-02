/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ptr::read_unaligned;

pub(crate) fn initialize(boot_info: usize, boot_magic: usize) -> bool {
    const MULTIBOOT_MAGIC: usize = 0x2bad_b002;
    const MEMORY_MAP_FLAG: u32 = 1 << 6;
    if boot_magic != MULTIBOOT_MAGIC || boot_info == 0 {
        return false;
    }

    let flags = unsafe { read_unaligned(boot_info as *const u32) };
    if flags & MEMORY_MAP_FLAG == 0 {
        return false;
    }
    let map_length = unsafe { read_unaligned((boot_info + 44) as *const u32) as usize };
    let map_start = unsafe { read_unaligned((boot_info + 48) as *const u32) as usize };
    let Some(map_end) = map_start.checked_add(map_length) else {
        return false;
    };
    if map_length == 0 {
        return false;
    }

    crate::platform::reset_memory_regions();
    let mut entry = map_start;
    while entry < map_end {
        if map_end - entry < 4 {
            return false;
        }
        let size = unsafe { read_unaligned(entry as *const u32) as usize };
        let Some(entry_length) = size.checked_add(4) else {
            return false;
        };
        if size < 20 || entry_length > map_end - entry {
            return false;
        }
        let start = unsafe { read_unaligned((entry + 4) as *const u64) };
        let length = unsafe { read_unaligned((entry + 12) as *const u64) };
        let kind = unsafe { read_unaligned((entry + 20) as *const u32) };
        if kind == 1 && !crate::platform::add_memory_region(start, length) {
            return false;
        }
        entry += entry_length;
    }
    !crate::platform::memory_regions().is_empty()
}