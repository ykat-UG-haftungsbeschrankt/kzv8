/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PciAddress {
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

pub(crate) fn read_config(address: PciAddress, offset: u8) -> Option<u32> {
    if offset & 3 != 0 {
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

pub(crate) fn find_device(vendor_id: u16, device_id: u16) -> Option<PciDevice> {
    for device in 0..32 {
        let address = PciAddress {
            bus: 0,
            device,
            function: 0,
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
    None
}