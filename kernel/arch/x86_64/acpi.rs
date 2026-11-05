/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

const RSDP_SIGNATURE: &[u8; 8] = b"RSD PTR ";
const MAX_TABLE_SIZE: usize = 1024 * 1024;

pub(crate) fn cpu_apic_ids() -> Option<Vec<u32>> {
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
        if table.get(..4)? == b"APIC" {
            return parse_madt(table);
        }
    }
    None
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

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    Some(u16::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    Some(u32::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let end = offset.checked_add(8)?;
    Some(u64::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
}