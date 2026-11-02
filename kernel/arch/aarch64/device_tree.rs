/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::scheduler::sync::IrqSpinLock;
use core::sync::atomic::{AtomicUsize, Ordering};

const MAX_CPU_IDS: usize = 8;

struct CpuIdList {
    ids: [u64; MAX_CPU_IDS],
    count: usize,
}

impl CpuIdList {
    const EMPTY: Self = Self {
        ids: [0; MAX_CPU_IDS],
        count: 0,
    };
}

static AARCH64_CPU_IDS: IrqSpinLock<CpuIdList> = IrqSpinLock::new(CpuIdList::EMPTY);
static PSCI_METHOD: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PsciConduit {
    Smc,
    Hvc,
}

fn reset_cpu_info() {
    *AARCH64_CPU_IDS.lock() = CpuIdList::EMPTY;
    PSCI_METHOD.store(0, Ordering::Release);
}

fn add_cpu_id(id: u64) -> bool {
    let mut cpu_ids = AARCH64_CPU_IDS.lock();
    if cpu_ids.count == MAX_CPU_IDS {
        return true;
    }
    let index = cpu_ids.count;
    cpu_ids.ids[index] = id;
    cpu_ids.count = index + 1;
    true
}

pub(crate) fn aarch64_cpu_ids() -> ([u64; MAX_CPU_IDS], usize) {
    let cpu_ids = AARCH64_CPU_IDS.lock();
    (cpu_ids.ids, cpu_ids.count)
}

pub(crate) fn psci_conduit() -> Option<PsciConduit> {
    match PSCI_METHOD.load(Ordering::Acquire) {
        1 => Some(PsciConduit::Smc),
        2 => Some(PsciConduit::Hvc),
        _ => None,
    }
}

fn fdt_u32(fdt: &[u8], total_size: usize, offset: usize) -> Option<u32> {
    if offset.checked_add(4)? > total_size {
        return None;
    }
    Some(u32::from_be_bytes(
        fdt.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn fdt_string_matches(
    fdt: &[u8],
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
    let Some(strings_end) = strings_offset.checked_add(strings_size) else {
        return false;
    };
    let Some(strings) = fdt.get(strings_offset..strings_end) else {
        return false;
    };
    let Some(actual_end) = name_end.checked_add(1) else {
        return false;
    };
    let Some(actual) = strings.get(offset..actual_end) else {
        return false;
    };
    actual[..expected.len()] == *expected && actual[expected.len()] == 0
}

fn fdt_value_matches(
    fdt: &[u8],
    total_size: usize,
    offset: usize,
    length: usize,
    value: &[u8],
) -> bool {
    let Some(end) = offset.checked_add(length) else {
        return false;
    };
    if end > total_size || length != value.len() + 1 {
        return false;
    }
    let Some(actual) = fdt.get(offset..end) else {
        return false;
    };
    actual[..value.len()] == *value && actual[value.len()] == 0
}

fn fdt_cells(fdt: &[u8], total_size: usize, offset: usize, count: usize) -> Option<u64> {
    if count == 0 || count > 2 {
        return None;
    }
    let mut value = 0u64;
    for index in 0..count {
        value = (value << 32) | u64::from(fdt_u32(fdt, total_size, offset + index * 4)?);
    }
    Some(value)
}

/// # Safety
/// `fdt` must point to a readable FDT blob for the duration of this call.
pub(crate) unsafe fn initialize(fdt_address: usize) -> bool {
    const FDT_MAGIC: u32 = 0xd00d_feed;
    const FDT_BEGIN_NODE: u32 = 1;
    const FDT_END_NODE: u32 = 2;
    const FDT_PROP: u32 = 3;
    const FDT_NOP: u32 = 4;
    const FDT_END: u32 = 9;

    if fdt_address == 0 {
        return false;
    }
    let header = unsafe { core::slice::from_raw_parts(fdt_address as *const u8, 40) };
    if fdt_u32(header, 40, 0) != Some(FDT_MAGIC) {
        return false;
    }
    let Some(total_size) = fdt_u32(header, 40, 4).map(|value| value as usize) else {
        return false;
    };
    if total_size < 40 || total_size > 1024 * 1024 {
        return false;
    }
    let fdt = unsafe { core::slice::from_raw_parts(fdt_address as *const u8, total_size) };
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

    crate::kernel::memory::resources::MemoryRegionRegistry::reset_memory_regions();
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
                    if fdt[cursor] == 0 {
                        break;
                    }
                    cursor += 1;
                }
                if cursor == structure_end {
                    return false;
                }
                let name_length = cursor - name_start;
                let name = &fdt[name_start..name_start + name_length];
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
                    && fdt_string_matches(fdt, strings_offset, strings_size, name_offset, b"method")
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
                    if length >= 7 && &fdt[cursor..cursor + 7] == b"memory\0" {
                        memory_nodes[node_depth - 1] = true;
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
                        if !crate::kernel::memory::resources::MemoryRegionRegistry::add_memory_region(address, size) {
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
    !crate::kernel::memory::resources::MemoryRegionRegistry::memory_regions().is_empty()
}
