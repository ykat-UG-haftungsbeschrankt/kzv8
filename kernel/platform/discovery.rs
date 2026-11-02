/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[cfg(target_arch = "x86_64")]
use core::ptr::read_unaligned;
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

#[cfg(target_arch = "aarch64")]
const MAX_CPU_IDS: usize = 8;
#[cfg(target_arch = "aarch64")]
static mut AARCH64_CPU_IDS: [u64; MAX_CPU_IDS] = [0; MAX_CPU_IDS];
#[cfg(target_arch = "aarch64")]
static AARCH64_CPU_COUNT: AtomicUsize = AtomicUsize::new(0);
#[cfg(target_arch = "aarch64")]
static PSCI_METHOD: AtomicUsize = AtomicUsize::new(0);

#[cfg(target_arch = "aarch64")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PsciConduit {
    Smc,
    Hvc,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VirtioWindow {
    pub(crate) start: usize,
    pub(crate) slots: usize,
    pub(crate) stride: usize,
}

fn reset_memory_regions() {
    unsafe {
        let regions = addr_of_mut!(MEMORY_REGIONS).cast::<MemoryRegion>();
        for index in 0..MAX_MEMORY_REGIONS {
            regions.add(index).write(MemoryRegion::EMPTY);
        }
    }
    MEMORY_REGION_COUNT.store(0, Ordering::Release);
}

#[cfg(target_arch = "aarch64")]
fn reset_cpu_info() {
    let ids = addr_of_mut!(AARCH64_CPU_IDS).cast::<u64>();
    for index in 0..MAX_CPU_IDS {
        unsafe { ids.add(index).write(0) };
    }
    AARCH64_CPU_COUNT.store(0, Ordering::Release);
    PSCI_METHOD.store(0, Ordering::Release);
}

#[cfg(target_arch = "aarch64")]
fn add_cpu_id(id: u64) -> bool {
    let count = AARCH64_CPU_COUNT.load(Ordering::Relaxed);
    if count == MAX_CPU_IDS {
        return true;
    }
    unsafe { addr_of_mut!(AARCH64_CPU_IDS).cast::<u64>().add(count).write(id) };
    AARCH64_CPU_COUNT.store(count + 1, Ordering::Release);
    true
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn aarch64_cpu_ids() -> &'static [u64] {
    let count = AARCH64_CPU_COUNT.load(Ordering::Acquire);
    unsafe { core::slice::from_raw_parts(addr_of!(AARCH64_CPU_IDS).cast::<u64>(), count) }
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn psci_conduit() -> Option<PsciConduit> {
    match PSCI_METHOD.load(Ordering::Acquire) {
        1 => Some(PsciConduit::Smc),
        2 => Some(PsciConduit::Hvc),
        _ => None,
    }
}

fn add_memory_region(start: u64, length: u64) -> bool {
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

#[cfg(target_arch = "x86_64")]
pub(crate) fn initialize_x86(boot_info: usize, boot_magic: usize) -> bool {
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

    reset_memory_regions();
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
        if kind == 1 && !add_memory_region(start, length) {
            return false;
        }
        entry += entry_length;
    }
    !memory_regions().is_empty()
}

#[cfg(target_arch = "aarch64")]
fn fdt_u32(base: usize, total_size: usize, offset: usize) -> Option<u32> {
    if offset.checked_add(4)? > total_size {
        return None;
    }
    let address = base.checked_add(offset)?;
    let bytes = unsafe { core::slice::from_raw_parts(address as *const u8, 4) };
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(target_arch = "aarch64")]
fn fdt_string_matches(
    base: usize,
    strings_offset: usize,
    strings_size: usize,
    name_offset: u32,
    expected: &[u8],
) -> bool {
    let offset = name_offset as usize;
    let Some(name_end) = offset.checked_add(expected.len()) else {
        return false;
    };
    if name_end >= strings_size {
        return false;
    }
    let Some(address) = base
        .checked_add(strings_offset)
        .and_then(|value| value.checked_add(offset))
    else {
        return false;
    };
    let actual = unsafe { core::slice::from_raw_parts(address as *const u8, expected.len() + 1) };
    actual[..expected.len()] == *expected && actual[expected.len()] == 0
}

#[cfg(target_arch = "aarch64")]
fn fdt_value_matches(base: usize, total_size: usize, offset: usize, length: usize, value: &[u8]) -> bool {
    let Some(end) = offset.checked_add(length) else {
        return false;
    };
    if end > total_size || length != value.len() + 1 {
        return false;
    }
    let Some(address) = base.checked_add(offset) else {
        return false;
    };
    let actual = unsafe { core::slice::from_raw_parts(address as *const u8, length) };
    actual[..value.len()] == *value && actual[value.len()] == 0
}

#[cfg(target_arch = "aarch64")]
fn fdt_cells(base: usize, total_size: usize, offset: usize, count: usize) -> Option<u64> {
    if count == 0 || count > 2 {
        return None;
    }
    let mut value = 0u64;
    for index in 0..count {
        value = (value << 32) | u64::from(fdt_u32(base, total_size, offset + index * 4)?);
    }
    Some(value)
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn initialize_aarch64(fdt: usize) -> bool {
    const FDT_MAGIC: u32 = 0xd00d_feed;
    const FDT_BEGIN_NODE: u32 = 1;
    const FDT_END_NODE: u32 = 2;
    const FDT_PROP: u32 = 3;
    const FDT_NOP: u32 = 4;
    const FDT_END: u32 = 9;

    if fdt == 0 || fdt_u32(fdt, 40, 0) != Some(FDT_MAGIC) {
        return false;
    }
    let Some(total_size) = fdt_u32(fdt, 40, 4).map(|value| value as usize) else {
        return false;
    };
    if total_size < 40 || total_size > 1024 * 1024 {
        return false;
    }
    let Some(structure_offset) = fdt_u32(fdt, total_size, 8).map(|value| value as usize) else {
        return false;
    };
    let Some(strings_offset) = fdt_u32(fdt, total_size, 12).map(|value| value as usize) else {
        return false;
    };
    let Some(strings_size) = fdt_u32(fdt, total_size, 32).map(|value| value as usize) else {
        return false;
    };
    let Some(structure_size) = fdt_u32(fdt, total_size, 36).map(|value| value as usize) else {
        return false;
    };
    let Some(structure_end) = structure_offset.checked_add(structure_size) else {
        return false;
    };
    let Some(strings_end) = strings_offset.checked_add(strings_size) else {
        return false;
    };
    if structure_end > total_size || strings_end > total_size {
        return false;
    }

    reset_memory_regions();
    reset_cpu_info();
    let mut address_cells = 2usize;
    let mut size_cells = 1usize;
    let mut cpu_address_cells = 1usize;
    let mut node_depth = 0usize;
    let mut memory_nodes = [false; 32];
    let mut cpus_nodes = [false; 32];
    let mut cpu_nodes = [false; 32];
    let mut psci_nodes = [false; 32];
    let mut cursor = structure_offset;
    while cursor < structure_end {
        let Some(token) = fdt_u32(fdt, structure_end, cursor) else {
            return false;
        };
        cursor += 4;
        match token {
            FDT_BEGIN_NODE => {
                if node_depth == memory_nodes.len() {
                    return false;
                }
                let name_start = cursor;
                while cursor < structure_end {
                    let Some(address) = fdt.checked_add(cursor) else {
                        return false;
                    };
                    if unsafe { core::ptr::read(address as *const u8) } == 0 {
                        break;
                    }
                    cursor += 1;
                }
                if cursor == structure_end {
                    return false;
                }
                let name_length = cursor - name_start;
                let name_address = fdt + name_start;
                let name =
                    unsafe { core::slice::from_raw_parts(name_address as *const u8, name_length) };
                memory_nodes[node_depth] =
                    node_depth == 1 && (name == b"memory" || name.starts_with(b"memory@"));
                cpus_nodes[node_depth] = node_depth == 1 && name == b"cpus";
                cpu_nodes[node_depth] = node_depth == 2
                    && cpus_nodes[node_depth - 1]
                    && (name == b"cpu" || name.starts_with(b"cpu@"));
                psci_nodes[node_depth] = node_depth == 1 && name == b"psci";
                node_depth += 1;
                let Some(next_cursor) =
                    cursor.checked_add(1).and_then(|value| value.checked_add(3))
                else {
                    return false;
                };
                cursor = next_cursor & !3;
            }
            FDT_END_NODE => {
                if node_depth == 0 {
                    return false;
                }
                node_depth -= 1;
            }
            FDT_PROP => {
                let Some(length) = fdt_u32(fdt, structure_end, cursor).map(|value| value as usize)
                else {
                    return false;
                };
                let Some(name_offset) = fdt_u32(fdt, structure_end, cursor + 4) else {
                    return false;
                };
                cursor += 8;
                let Some(property_end) = cursor.checked_add(length) else {
                    return false;
                };
                if property_end > structure_end {
                    return false;
                }
                if node_depth == 1
                    && fdt_string_matches(
                        fdt,
                        strings_offset,
                        strings_size,
                        name_offset,
                        b"#address-cells",
                    )
                {
                    address_cells = fdt_u32(fdt, structure_end, cursor)
                        .map(|value| value as usize)
                        .unwrap_or(0);
                } else if node_depth == 1
                    && fdt_string_matches(
                        fdt,
                        strings_offset,
                        strings_size,
                        name_offset,
                        b"#size-cells",
                    )
                {
                    size_cells = fdt_u32(fdt, structure_end, cursor)
                        .map(|value| value as usize)
                        .unwrap_or(0);
                } else if node_depth == 2
                    && cpus_nodes[node_depth - 1]
                    && fdt_string_matches(
                        fdt,
                        strings_offset,
                        strings_size,
                        name_offset,
                        b"#address-cells",
                    )
                {
                    cpu_address_cells = fdt_u32(fdt, structure_end, cursor)
                        .map(|value| value as usize)
                        .unwrap_or(0);
                } else if node_depth == 2
                    && psci_nodes[node_depth - 1]
                    && fdt_string_matches(
                        fdt,
                        strings_offset,
                        strings_size,
                        name_offset,
                        b"method",
                    )
                {
                    if fdt_value_matches(fdt, total_size, cursor, length, b"smc") {
                        PSCI_METHOD.store(1, Ordering::Release);
                    } else if fdt_value_matches(fdt, total_size, cursor, length, b"hvc") {
                        PSCI_METHOD.store(2, Ordering::Release);
                    }
                } else if node_depth > 1
                    && fdt_string_matches(
                        fdt,
                        strings_offset,
                        strings_size,
                        name_offset,
                        b"device_type",
                    )
                {
                    if length >= 7 {
                        let Some(value_address) = fdt.checked_add(cursor) else {
                            return false;
                        };
                        let value =
                            unsafe { core::slice::from_raw_parts(value_address as *const u8, 7) };
                        if value == b"memory\0" {
                            memory_nodes[node_depth - 1] = true;
                        }
                    }
                } else if node_depth > 2
                    && cpu_nodes[node_depth - 1]
                    && fdt_string_matches(fdt, strings_offset, strings_size, name_offset, b"reg")
                {
                    if cpu_address_cells != 0
                        && cpu_address_cells <= 2
                        && length == cpu_address_cells * 4
                    {
                        if let Some(cpu_id) =
                            fdt_cells(fdt, structure_end, cursor, cpu_address_cells)
                        {
                            let _ = add_cpu_id(cpu_id);
                        }
                    }
                } else if node_depth > 1
                    && memory_nodes[node_depth - 1]
                    && fdt_string_matches(fdt, strings_offset, strings_size, name_offset, b"reg")
                {
                    let Some(tuple_size) = (address_cells + size_cells).checked_mul(4) else {
                        return false;
                    };
                    if address_cells == 0
                        || address_cells > 2
                        || size_cells == 0
                        || size_cells > 2
                        || tuple_size == 0
                        || length % tuple_size != 0
                    {
                        return false;
                    }
                    let mut reg_offset = 0;
                    while reg_offset < length {
                        let address =
                            fdt_cells(fdt, structure_end, cursor + reg_offset, address_cells);
                        let size = fdt_cells(
                            fdt,
                            structure_end,
                            cursor + reg_offset + address_cells * 4,
                            size_cells,
                        );
                        let (Some(address), Some(size)) = (address, size) else {
                            return false;
                        };
                        if !add_memory_region(address, size) {
                            return false;
                        }
                        reg_offset += tuple_size;
                    }
                }
                let Some(next_cursor) = property_end.checked_add(3) else {
                    return false;
                };
                cursor = next_cursor & !3;
            }
            FDT_NOP => {}
            FDT_END => break,
            _ => return false,
        }
    }
    !memory_regions().is_empty()
}

#[cfg(target_arch = "x86_64")]
pub(crate) const fn virtio_window() -> VirtioWindow {
    VirtioWindow {
        start: 0xfeb0_0000,
        slots: 24,
        stride: 0x200,
    }
}

#[cfg(target_arch = "aarch64")]
pub(crate) const fn virtio_window() -> VirtioWindow {
    VirtioWindow {
        start: 0x0a00_0000,
        slots: 32,
        stride: 0x200,
    }
}

pub(crate) unsafe fn find_virtio_device(device_id: u32) -> Option<usize> {
    let window = virtio_window();
    for slot in 0..window.slots {
        let base = window.start + slot * window.stride;
        let magic = core::ptr::read_volatile(base as *const u32);
        let version = core::ptr::read_volatile((base + 4) as *const u32);
        let id = core::ptr::read_volatile((base + 8) as *const u32);
        if magic == 0x7472_6976 && version == 2 && id == device_id {
            return Some(base);
        }
    }
    None
}
