/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VtdUnitCapabilities {
    pub(crate) segment: u16,
    pub(crate) register_base: u64,
    pub(crate) version: u32,
    pub(crate) host_address_width: u8,
    pub(crate) supported_address_widths: u8,
    pub(crate) supports_2m_pages: bool,
    pub(crate) supports_1g_pages: bool,
    pub(crate) queued_invalidation: bool,
    pub(crate) interrupt_remapping: bool,
    pub(crate) pass_through: bool,
}

pub(crate) fn probe_units() -> Option<Vec<VtdUnitCapabilities>> {
    let dmar = super::acpi::dmar_info()?;
    let mut units = Vec::with_capacity(dmar.units.len());
    for unit in dmar.units {
        let base = usize::try_from(unit.register_base).ok()?;
        let version = super::paging::read_mmio_u32(base)?;
        let capability = super::paging::read_mmio_u64(base.checked_add(8)?)?;
        let extended_capability = super::paging::read_mmio_u64(base.checked_add(16)?)?;
        if version == 0 || version == u32::MAX {
            return None;
        }
        units.push(VtdUnitCapabilities {
            segment: unit.segment,
            register_base: unit.register_base,
            version,
            host_address_width: (((capability >> 16) & 0x3f) + 1) as u8,
            supported_address_widths: ((capability >> 8) & 0x1f) as u8,
            supports_2m_pages: capability & (1 << 34) != 0,
            supports_1g_pages: capability & (1 << 35) != 0,
            queued_invalidation: extended_capability & (1 << 1) != 0,
            interrupt_remapping: extended_capability & (1 << 3) != 0,
            pass_through: extended_capability & (1 << 6) != 0,
        });
    }
    (!units.is_empty()).then_some(units)
}