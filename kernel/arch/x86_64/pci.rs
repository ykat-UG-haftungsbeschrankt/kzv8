/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PciAddress {
    pub(crate) segment: u16,
    pub(crate) bus: u8,
    pub(crate) device: u8,
    pub(crate) function: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PciDevice {
    pub(crate) address: PciAddress,
    pub(crate) vendor_id: u16,
    pub(crate) device_id: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SriovCapability {
    pub(crate) offset: u16,
    pub(crate) enabled: bool,
    pub(crate) initial_vfs: u16,
    pub(crate) total_vfs: u16,
    pub(crate) num_vfs: u16,
    pub(crate) first_vf_offset: u16,
    pub(crate) vf_stride: u16,
    pub(crate) vf_device_id: u16,
    pub(crate) supported_page_sizes: u32,
    pub(crate) system_page_size: u32,
}

pub(crate) fn read_config(address: PciAddress, offset: u8) -> Option<u32> {
    if address.segment != 0 || offset & 3 != 0 {
        return None;
    }
    let selector = 0x8000_0000
        | (u32::from(address.bus) << 16)
        | (u32::from(address.device) << 11)
        | (u32::from(address.function) << 8)
        | u32::from(offset & 0xfc);
    let value: u32;
    unsafe {
        core::arch::asm!(
            "out dx, eax",
            in("dx") 0x0cf8u16,
            in("eax") selector,
            options(nomem, nostack, preserves_flags),
        );
        core::arch::asm!(
            "in eax, dx",
            in("dx") 0x0cfcu16,
            out("eax") value,
            options(nomem, nostack, preserves_flags),
        );
    }
    Some(value)
}

pub(crate) fn read_config_extended(address: PciAddress, offset: u16) -> Option<u32> {
    let allocations = super::acpi::mcfg_allocations()?;
    read_ecam_config(&allocations, address, offset)
}

pub(crate) fn find_extended_capability(address: PciAddress, capability_id: u16) -> Option<u16> {
    let allocations = super::acpi::mcfg_allocations()?;
    find_extended_capability_in(&allocations, address, capability_id)
}

pub(crate) fn sriov_capability(address: PciAddress) -> Option<SriovCapability> {
    const SR_IOV_CAPABILITY_ID: u16 = 0x10;
    let allocations = super::acpi::mcfg_allocations()?;
    let offset = find_extended_capability_in(&allocations, address, SR_IOV_CAPABILITY_ID)?;
    let control_and_status = read_ecam_config(&allocations, address, offset.checked_add(8)?)?;
    let vf_counts = read_ecam_config(&allocations, address, offset.checked_add(12)?)?;
    let vf_count = read_ecam_config(&allocations, address, offset.checked_add(16)?)?;
    let vf_offset_and_stride = read_ecam_config(&allocations, address, offset.checked_add(20)?)?;
    let vf_device_id = read_ecam_config(&allocations, address, offset.checked_add(24)?)?;
    let page_sizes = read_ecam_config(&allocations, address, offset.checked_add(28)?)?;
    let system_page_size = read_ecam_config(&allocations, address, offset.checked_add(32)?)?;
    Some(SriovCapability {
        offset,
        enabled: control_and_status & 1 != 0,
        initial_vfs: vf_counts as u16,
        total_vfs: (vf_counts >> 16) as u16,
        num_vfs: vf_count as u16,
        first_vf_offset: vf_offset_and_stride as u16,
        vf_stride: (vf_offset_and_stride >> 16) as u16,
        vf_device_id: vf_device_id as u16,
        supported_page_sizes: page_sizes,
        system_page_size,
    })
}

fn find_extended_capability_in(
    allocations: &[super::acpi::McfgAllocation],
    address: PciAddress,
    capability_id: u16,
) -> Option<u16> {
    let mut offset = 0x100u16;
    for _ in 0..((4096 - 0x100) / 4) {
        let header = read_ecam_config(allocations, address, offset)?;
        if header == 0 || header == u32::MAX {
            return None;
        }
        if header as u16 == capability_id {
            return Some(offset);
        }
        let next = ((header >> 20) & 0x0fff) as u16;
        if next == 0 || next < 0x100 || next > 0xffc || next & 3 != 0 {
            return None;
        }
        offset = next;
    }
    None
}

fn read_ecam_config(
    allocations: &[super::acpi::McfgAllocation],
    address: PciAddress,
    offset: u16,
) -> Option<u32> {
    const PAGE_SIZE: usize = 4096;
    if offset & 3 != 0
        || offset > 0xffc
        || address.device >= 32
        || address.function >= 8
    {
        return None;
    }
    let allocation = allocations.iter().find(|allocation| {
        allocation.segment == address.segment
            && (allocation.start_bus..=allocation.end_bus).contains(&address.bus)
    })?;
    let bus_offset = u64::from(address.bus - allocation.start_bus).checked_shl(20)?;
    let function_offset = (u64::from(address.device) << 15)
        | (u64::from(address.function) << 12)
        | u64::from(offset);
    let physical_address = allocation
        .base
        .checked_add(bus_offset)?
        .checked_add(function_offset)?;
    let physical_address = usize::try_from(physical_address).ok()?;
    let page_physical_address = physical_address & !(PAGE_SIZE - 1);
    let page_offset = physical_address & (PAGE_SIZE - 1);
    super::paging::read_mmio_u32(page_physical_address.checked_add(page_offset)?)
}

pub(crate) fn find_device(vendor_id: u16, device_id: u16) -> Option<PciDevice> {
    for device in 0..32 {
        let address = PciAddress {
            segment: 0,
            bus: 0,
            device,
            function: 0,
        };
        let Some(identity) = read_config(address, 0) else {
            continue;
        };
        let header_type = read_config(address, 0x0c).map(|value| (value >> 16) as u8)?;
        let function_count = if header_type & 0x80 != 0 { 8 } else { 1 };
        for function in 0..function_count {
            let address = PciAddress {
                function,
                ..address
            };
            let Some(identity) = read_config(address, 0) else {
                continue;
            };
            let vendor = identity as u16;
            let id = (identity >> 16) as u16;
            if vendor == vendor_id && id == device_id {
                return Some(PciDevice {
                    address,
                    vendor_id: vendor,
                    device_id: id,
                });
            }
        }
    }
    None
}
