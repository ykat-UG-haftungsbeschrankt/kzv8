/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use crate::kernel::lib::endian::{read_u16, read_u32, read_u64};

const RSDP_SIGNATURE: &[u8; 8] = b"RSD PTR ";
const MAX_TABLE_SIZE: usize = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DmarInfo {
    pub(crate) host_address_width: u8,
    pub(crate) interrupt_remapping: bool,
    pub(crate) x2apic_opt_out: bool,
    //@FIXME: INDEXTYPE(units); parsed firmware descriptor list; Vec append and iteration are appropriate, keep Vec.
    pub(crate) units: Vec<DmarUnit>,
    //@FIXME: INDEXTYPE(reserved_regions); parsed firmware descriptor list; keep Vec unless keyed lookup becomes hot.
    pub(crate) reserved_regions: Vec<DmarReservedRegion>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DmarUnit {
    pub(crate) segment: u16,
    pub(crate) register_base: u64,
    pub(crate) include_all: bool,
    //@FIXME: INDEXTYPE(device_scopes); parsed ordered descriptor list; keep Vec.
    pub(crate) device_scopes: Vec<DmarDeviceScope>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DmarReservedRegion {
    pub(crate) segment: u16,
    pub(crate) base: u64,
    pub(crate) limit: u64,
    //@FIXME: INDEXTYPE(device_scopes); parsed ordered descriptor list; keep Vec.
    pub(crate) device_scopes: Vec<DmarDeviceScope>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DmarDeviceScope {
    pub(crate) scope_type: u8,
    pub(crate) enumeration_id: u8,
    pub(crate) start_bus: u8,
    //@FIXME: INDEXTYPE(path); short ordered PCI path; Vec indexing and append are O(1) amortized, keep Vec.
    pub(crate) path: Vec<DmarPathEntry>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DmarPathEntry {
    pub(crate) device: u8,
    pub(crate) function: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct McfgAllocation {
    pub(crate) base: u64,
    pub(crate) segment: u16,
    pub(crate) start_bus: u8,
    pub(crate) end_bus: u8,
}

pub(crate) fn cpu_apic_ids() -> Option<Vec<u32>> {
    parse_madt(find_acpi_table(b"APIC")?)
}

pub(crate) fn mcfg_allocations() -> Option<Vec<McfgAllocation>> {
    let table = find_acpi_table(b"MCFG")?;
    if table.len() < 44 || (table.len() - 44) % 16 != 0 {
        return None;
    }
    let mut allocations = Vec::new();
    for allocation in table[44..].chunks_exact(16) {
        let base = read_u64(allocation, 0)?;
        let segment = read_u16(allocation, 8)?;
        let start_bus = allocation[10];
        let end_bus = allocation[11];
        if base == 0 || base % (1 << 20) != 0 || start_bus > end_bus {
            return None;
        }
        let bus_count = u64::from(end_bus - start_bus) + 1;
        let region_length = bus_count.checked_mul(1 << 20)?;
        base.checked_add(region_length)?;
        allocations.push(McfgAllocation {
            base,
            segment,
            start_bus,
            end_bus,
        });
    }
    (!allocations.is_empty()).then_some(allocations)
}

pub(crate) fn dmar_info() -> Option<DmarInfo> {
    let table = find_acpi_table(b"DMAR")?;
    if table.len() < 48 {
        return None;
    }
    let host_address_width = table[36].checked_add(1)?;
    if !(32..=64).contains(&host_address_width) {
        return None;
    }
    let flags = table[37];
    let mut units = Vec::new();
    let mut reserved_regions = Vec::new();
    let mut offset = 48;
    while offset < table.len() {
        let remaining = table.len() - offset;
        if remaining < 4 {
            return None;
        }
        let structure_type = read_u16(table, offset)?;
        let structure_length = usize::from(read_u16(table, offset + 2)?);
        if structure_length < 4 || structure_length > remaining {
            return None;
        }
        let structure = table.get(offset..offset + structure_length)?;
        match structure_type {
            0 => {
                if structure_length < 16 {
                    return None;
                }
                let register_base = read_u64(structure, 8)?;
                if register_base == 0 || register_base % 4096 != 0 {
                    return None;
                }
                units.push(DmarUnit {
                    segment: read_u16(structure, 6)?,
                    register_base,
                    include_all: structure[4] & 1 != 0,
                    device_scopes: parse_device_scopes(structure, 16)?,
                });
            }
            1 => {
                if structure_length < 24 {
                    return None;
                }
                let base = read_u64(structure, 8)?;
                let limit = read_u64(structure, 16)?;
                if base > limit {
                    return None;
                }
                reserved_regions.push(DmarReservedRegion {
                    segment: read_u16(structure, 6)?,
                    base,
                    limit,
                    device_scopes: parse_device_scopes(structure, 24)?,
                });
            }
            _ => {}
        }
        offset += structure_length;
    }
    (!units.is_empty()).then_some(DmarInfo {
        host_address_width,
        interrupt_remapping: flags & 1 != 0,
        x2apic_opt_out: flags & (1 << 1) != 0,
        units,
        reserved_regions,
    })
}

fn find_acpi_table(signature: &[u8; 4]) -> Option<&'static [u8]> {
    let rsdp_address = find_rsdp()?;
    let rsdp = physical_bytes(rsdp_address as u64, 20)?;
    let revision = *rsdp.get(15)?;
    let rsdt_address = u64::from(read_u32(rsdp, 16)?);
    let (root_address, root_signature, entry_size) = if revision >= 2 {
        let extended_rsdp = physical_bytes(rsdp_address as u64, 36)?;
        let xsdt_address = read_u64(extended_rsdp, 24)?;
        if xsdt_address != 0 {
            (xsdt_address, b"XSDT" as &[u8], 8)
        } else {
            (rsdt_address, b"RSDT" as &[u8], 4)
        }
    } else {
        (rsdt_address, b"RSDT" as &[u8], 4)
    };
    let root = read_sdt(root_address)?;
    if root.get(..4)? != root_signature {
        return None;
    }
    let entries = root.get(36..)?;
    if entries.len() % entry_size != 0 {
        return None;
    }
    for entry in entries.chunks_exact(entry_size) {
        let table_address = if entry_size == 8 {
            read_u64(entry, 0)?
        } else {
            u64::from(read_u32(entry, 0)?)
        };
        let Some(table) = read_sdt(table_address) else {
            continue;
        };
        if table.get(..4)? == signature.as_slice() {
            return Some(table);
        }
    }
    None
}

fn parse_device_scopes(structure: &[u8], mut offset: usize) -> Option<Vec<DmarDeviceScope>> {
    let mut scopes = Vec::new();
    while offset < structure.len() {
        let remaining = structure.len().checked_sub(offset)?;
        if remaining < 6 {
            return None;
        }
        let scope_length = usize::from(*structure.get(offset + 1)?);
        if scope_length < 8 || scope_length > remaining || (scope_length - 6) % 2 != 0 {
            return None;
        }
        let path_bytes = structure.get(offset + 6..offset + scope_length)?;
        let mut path = Vec::new();
        for entry in path_bytes.chunks_exact(2) {
            path.push(DmarPathEntry {
                device: entry[0],
                function: entry[1],
            });
        }
        scopes.push(DmarDeviceScope {
            scope_type: structure[offset],
            enumeration_id: structure[offset + 4],
            start_bus: structure[offset + 5],
            path,
        });
        offset = offset.checked_add(scope_length)?;
    }
    Some(scopes)
}

fn find_rsdp() -> Option<usize> {
    if let Some(ebda_segment) = physical_bytes(0x40e, 2).and_then(|bytes| read_u16(bytes, 0)) {
        let ebda = usize::from(ebda_segment) << 4;
        if (0x400..0xa0000).contains(&ebda) {
            if let Some(address) = scan_rsdp(ebda, ebda + 1024) {
                return Some(address);
            }
        }
    }
    scan_rsdp(0xe0000, 0x100000)
}

fn scan_rsdp(start: usize, end: usize) -> Option<usize> {
    let mut address = start.checked_add(15)? & !15;
    while address.checked_add(20).is_some_and(|next| next <= end) {
        if valid_rsdp(address) {
            return Some(address);
        }
        address = address.checked_add(16)?;
    }
    None
}

fn valid_rsdp(address: usize) -> bool {
    let Some(rsdp) = physical_bytes(address as u64, 20) else {
        return false;
    };
    if rsdp.get(..8) != Some(RSDP_SIGNATURE.as_slice()) || checksum(rsdp) != 0 {
        return false;
    }
    if rsdp[15] < 2 {
        return true;
    }
    let Some(extended_rsdp) = physical_bytes(address as u64, 36) else {
        return false;
    };
    let Some(length) = read_u32(extended_rsdp, 20).map(|value| value as usize) else {
        return false;
    };
    length >= 36
        && length <= 4096
        && physical_bytes(address as u64, length).is_some_and(|bytes| checksum(bytes) == 0)
}

fn read_sdt(address: u64) -> Option<&'static [u8]> {
    let header = physical_bytes(address, 36)?;
    let length = read_u32(header, 4)? as usize;
    if !(36..=MAX_TABLE_SIZE).contains(&length) {
        return None;
    }
    let table = physical_bytes(address, length)?;
    (checksum(table) == 0).then_some(table)
}

fn parse_madt(table: &[u8]) -> Option<Vec<u32>> {
    if table.len() < 44 {
        return None;
    }
    let mut apic_ids = Vec::new();
    let mut offset = 44;
    while offset < table.len() {
        let remaining = table.len() - offset;
        if remaining < 2 {
            return None;
        }
        let record_type = table[offset];
        let record_length = usize::from(table[offset + 1]);
        if record_length < 2 || record_length > remaining {
            return None;
        }
        match record_type {
            0 if record_length >= 8 => {
                let flags = read_u32(table, offset + 4)?;
                if flags & 0x3 != 0 {
                    apic_ids.push(u32::from(table[offset + 3]));
                }
            }
            9 if record_length >= 16 => {
                let apic_id = read_u32(table, offset + 4)?;
                let flags = read_u32(table, offset + 8)?;
                if flags & 0x3 != 0 {
                    apic_ids.push(apic_id);
                }
            }
            _ => {}
        }
        offset += record_length;
    }
    apic_ids.sort_unstable();
    apic_ids.dedup();
    (!apic_ids.is_empty()).then_some(apic_ids)
}

fn physical_bytes(address: u64, length: usize) -> Option<&'static [u8]> {
    if length == 0 {
        return None;
    }
    let address = usize::try_from(address).ok()?;
    let end = address.checked_add(length)?;
    if !super::ram_map::is_identity_mapped(end - 1) {
        return None;
    }
    // SAFETY: the x86 early identity map covers this checked physical range.
    Some(unsafe { core::slice::from_raw_parts(address as *const u8, length) })
}

fn checksum(bytes: &[u8]) -> u8 {
    bytes
        .iter()
        .fold(0, |sum, byte| sum.wrapping_add(*byte))
}
