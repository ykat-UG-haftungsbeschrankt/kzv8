/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::arch::traits::TimerArchitecture as _;

static CONFIG_LOCK: IrqSpinLock<()> = IrqSpinLock::new(());

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
pub(crate) struct PciBar {
    pub(crate) index: u8,
    pub(crate) address: u64,
    pub(crate) length: u64,
}

pub(crate) struct PreparedPciFunction {
    address: PciAddress,
}

impl PreparedPciFunction {
    pub(crate) const fn address(&self) -> PciAddress {
        self.address
    }
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
    if address.segment != 0
        || address.device >= 32
        || address.function >= 8
        || offset & 3 != 0
    {
        return None;
    }
    let _guard = CONFIG_LOCK.lock();
    Some(read_config_unlocked(address, offset))
}

pub(crate) fn read_config_u16(address: PciAddress, offset: u8) -> Option<u16> {
    if offset & 1 != 0 {
        return None;
    }
    let value = read_config(address, offset & !3)?;
    Some((value >> ((offset & 2) * 8)) as u16)
}

pub(crate) fn write_config(address: PciAddress, offset: u8, value: u32) -> Option<()> {
    if address.segment != 0
        || address.device >= 32
        || address.function >= 8
        || offset & 3 != 0
    {
        return None;
    }
    let _guard = CONFIG_LOCK.lock();
    write_config_unlocked(address, offset, value);
    Some(())
}

pub(crate) fn write_config_u16(address: PciAddress, offset: u8, value: u16) -> Option<()> {
    if address.segment != 0
        || address.device >= 32
        || address.function >= 8
        || offset & 1 != 0
    {
        return None;
    }
    let _guard = CONFIG_LOCK.lock();
    write_config_u16_unlocked(address, offset, value);
    Some(())
}

pub(crate) fn set_bus_mastering(address: PciAddress, enabled: bool) -> Option<bool> {
    const PCI_COMMAND: u8 = 0x04;
    const BUS_MASTER_ENABLE: u16 = 1 << 2;
    let command = read_config_u16(address, PCI_COMMAND)?;
    if command == u16::MAX {
        return None;
    }
    let was_enabled = command & BUS_MASTER_ENABLE != 0;
    let updated = if enabled {
        command | BUS_MASTER_ENABLE
    } else {
        command & !BUS_MASTER_ENABLE
    };
    if updated != command {
        write_config_u16(address, PCI_COMMAND, updated)?;
        if read_config_u16(address, PCI_COMMAND)? & BUS_MASTER_ENABLE
            != updated & BUS_MASTER_ENABLE
        {
            return None;
        }
    }
    Some(was_enabled)
}

pub(crate) fn activate_memory_and_bus_mastering(address: PciAddress) -> Option<()> {
    const PCI_COMMAND: u8 = 0x04;
    const MEMORY_SPACE_ENABLE: u16 = 1 << 1;
    const BUS_MASTER_ENABLE: u16 = 1 << 2;
    let command = read_config_u16(address, PCI_COMMAND)?;
    if command == u16::MAX {
        return None;
    }
    let updated = command | MEMORY_SPACE_ENABLE | BUS_MASTER_ENABLE;
    if updated != command {
        write_config_u16(address, PCI_COMMAND, updated)?;
    }
    let verified = read_config_u16(address, PCI_COMMAND)?;
    (verified & (MEMORY_SPACE_ENABLE | BUS_MASTER_ENABLE)
        == MEMORY_SPACE_ENABLE | BUS_MASTER_ENABLE)
        .then_some(())
}

pub(crate) fn prepare_for_assignment(
    address: PciAddress,
    assigned: &[PciAddress],
) -> Option<PreparedPciFunction> {
    let header = read_config(address, 0x0c)?;
    let command = read_config_u16(address, 0x04)?;
    if address.bus != 0
        || address.function != 0
        || header & (1 << 23) != 0
        || ((header >> 16) as u8 & 0x7f) != 0
        || command & 0x7 != 0
        || !no_unassigned_bus_mastering(address, assigned)?
        || !supports_function_level_reset(address)?
    {
        return None;
    }
    reset_and_mask(address)?;
    Some(PreparedPciFunction { address })
}

pub(crate) fn quiesce_for_detach(address: PciAddress) -> Option<()> {
    if !supports_function_level_reset(address)? {
        return None;
    }
    reset_and_mask(address)
}

fn supports_function_level_reset(address: PciAddress) -> Option<bool> {
    let pcie_capability = find_standard_capability(address, 0x10)?;
    let device_capabilities = read_config(address, pcie_capability.checked_add(4)?)?;
    Some(device_capabilities & (1 << 28) != 0)
}

fn reset_and_mask(address: PciAddress) -> Option<()> {
    disable_bus_mastering(address)?;
    mask_interrupts(address)?;
    let pcie_capability = find_standard_capability(address, 0x10)?;
    let device_capabilities = read_config(address, pcie_capability.checked_add(4)?)?;
    if device_capabilities & (1 << 28) == 0 {
        return None;
    }
    let device_control_offset = pcie_capability.checked_add(8)?;
    let device_control = read_config_u16(address, device_control_offset)?;
    write_config_u16(address, device_control_offset, device_control | (1 << 15))?;

    let start = crate::kernel::arch::TimerArchitecture::now_ns()
        .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns);
    loop {
        let now = crate::kernel::arch::TimerArchitecture::now_ns()
            .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns);
        if now.wrapping_sub(start) >= 100_000_000 {
            break;
        }
        core::hint::spin_loop();
    }
    let identity = read_config(address, 0)?;
    if identity as u16 == 0 || identity as u16 == u16::MAX {
        return None;
    }
    disable_bus_mastering(address)?;
    mask_interrupts(address)?;
    Some(())
}

fn mask_interrupts(address: PciAddress) -> Option<()> {
    const PCI_COMMAND: u8 = 0x04;
    const PCI_STATUS: u8 = 0x06;
    const PCI_STATUS_CAPABILITIES: u16 = 1 << 4;
    const PCI_COMMAND_INTERRUPT_DISABLE: u16 = 1 << 10;

    let command = read_config_u16(address, PCI_COMMAND)?;
    write_config_u16(
        address,
        PCI_COMMAND,
        command | PCI_COMMAND_INTERRUPT_DISABLE,
    )?;
    let status = read_config_u16(address, PCI_STATUS)?;
    if status & PCI_STATUS_CAPABILITIES == 0 {
        return Some(());
    }
    let mut capability = read_config_u8(address, 0x34)? & !3;
    let mut visited = 0u64;
    for _ in 0..48 {
        if capability == 0 {
            return Some(());
        }
        if !(0x40..=0xfc).contains(&capability) || capability & 3 != 0 {
            return None;
        }
        let slot = (capability - 0x40) / 4;
        let bit = 1u64 << slot;
        if visited & bit != 0 {
            return None;
        }
        visited |= bit;

        let header = read_config(address, capability)?;
        let identifier = header as u8;
        let next = (header >> 8) as u8 & !3;
        let control_offset = capability.checked_add(2)?;
        if identifier == 0x05 {
            let control = read_config_u16(address, control_offset)?;
            write_config_u16(address, control_offset, control & !1)?;
        } else if identifier == 0x11 {
            let control = read_config_u16(address, control_offset)?;
            write_config_u16(
                address,
                control_offset,
                (control & !(1 << 15)) | (1 << 14),
            )?;
        }
        capability = next;
    }
    None
}

fn find_standard_capability(address: PciAddress, identifier: u8) -> Option<u8> {
    if read_config_u16(address, 0x06)? & (1 << 4) == 0 {
        return None;
    }
    let mut capability = read_config_u8(address, 0x34)? & !3;
    let mut visited = 0u64;
    for _ in 0..48 {
        if capability == 0 {
            return None;
        }
        if !(0x40..=0xfc).contains(&capability) || capability & 3 != 0 {
            return None;
        }
        let slot = (capability - 0x40) / 4;
        let bit = 1u64 << slot;
        if visited & bit != 0 {
            return None;
        }
        visited |= bit;
        let header = read_config(address, capability)?;
        if header as u8 == identifier {
            return Some(capability);
        }
        capability = (header >> 8) as u8 & !3;
    }
    None
}

fn read_config_u8(address: PciAddress, offset: u8) -> Option<u8> {
    let value = read_config(address, offset & !3)?;
    Some((value >> (u32::from(offset & 3) * 8)) as u8)
}

unsafe fn legacy_read_config(address: PciAddress, offset: u8) -> u32 {
    let selector = 0x8000_0000
        | (u32::from(address.bus) << 16)
        | (u32::from(address.device) << 11)
        | (u32::from(address.function) << 8)
        | u32::from(offset & 0xfc);
    let value: u32;
    // SAFETY: callers validate the BDF and hold CONFIG_LOCK across this CF8/CFC transaction.
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
    value
}

fn read_config_unlocked(address: PciAddress, offset: u8) -> u32 {
    // SAFETY: callers hold CONFIG_LOCK and validate the BDF and aligned offset.
    unsafe { legacy_read_config(address, offset) }
}

fn read_config_u16_unlocked(address: PciAddress, offset: u8) -> u16 {
    let value = read_config_unlocked(address, offset & !3);
    (value >> ((offset & 2) * 8)) as u16
}

fn write_config_unlocked(address: PciAddress, offset: u8, value: u32) {
    let selector = 0x8000_0000
        | (u32::from(address.bus) << 16)
        | (u32::from(address.device) << 11)
        | (u32::from(address.function) << 8)
        | u32::from(offset);
    // SAFETY: callers hold CONFIG_LOCK and validate the BDF and aligned offset.
    unsafe {
        core::arch::asm!(
            "out dx, eax",
            in("dx") 0x0cf8u16,
            in("eax") selector,
            options(nomem, nostack, preserves_flags),
        );
        core::arch::asm!(
            "out dx, eax",
            in("dx") 0x0cfcu16,
            in("eax") value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

fn write_config_u16_unlocked(address: PciAddress, offset: u8, value: u16) {
    let selector = 0x8000_0000
        | (u32::from(address.bus) << 16)
        | (u32::from(address.device) << 11)
        | (u32::from(address.function) << 8)
        | u32::from(offset & !3);
    let port = 0x0cfcu16 + u16::from(offset & 2);
    // SAFETY: callers hold CONFIG_LOCK and validate the BDF and even offset.
    unsafe {
        core::arch::asm!(
            "out dx, eax",
            in("dx") 0x0cf8u16,
            in("eax") selector,
            options(nomem, nostack, preserves_flags),
        );
        core::arch::asm!(
            "out dx, ax",
            in("dx") port,
            in("ax") value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

pub(crate) fn disable_bus_mastering(address: PciAddress) -> Option<bool> {
    set_bus_mastering(address, false)
}

pub(crate) fn no_unassigned_bus_mastering(
    target: PciAddress,
    assigned: &[PciAddress],
) -> Option<bool> {
    if target.segment != 0 {
        return None;
    }
    for bus in 0u16..=u16::from(u8::MAX) {
        for device in 0u8..32 {
            let function_zero = PciAddress {
                segment: 0,
                bus: bus as u8,
                device,
                function: 0,
            };
            let identity = read_config(function_zero, 0)?;
            if identity as u16 == 0 || identity as u16 == u16::MAX {
                continue;
            }
            let header = read_config(function_zero, 0x0c)?;
            let function_count = if (header >> 23) & 1 != 0 { 8 } else { 1 };
            for function in 0..function_count {
                let address = PciAddress {
                    function,
                    ..function_zero
                };
                let function_identity = if function == 0 {
                    identity
                } else {
                    read_config(address, 0)?
                };
                if function_identity as u16 == 0 || function_identity as u16 == u16::MAX {
                    continue;
                }
                if address != target
                    && !assigned.contains(&address)
                    && read_config_u16(address, 0x04)? & (1 << 2) != 0
                {
                    return Some(false);
                }
            }
        }
    }
    Some(true)
}

pub(crate) fn find_guest_mmio_bar(address: PciAddress, maximum_length: u64) -> Option<PciBar> {
    let header_type = (read_config(address, 0x0c)? >> 16) as u8 & 0x7f;
    if header_type != 0 {
        return None;
    }
    let mut index = 0;
    while index < 6 {
        let (bar, is_64_bit) = probe_memory_bar(address, index)?;
        if let Some(bar) = bar {
            let end = bar.address.checked_add(bar.length)?;
            let memory_regions = crate::kernel::memory::memblock::MemBlock::memory_regions();
            if bar.length >= 4096
                && bar.length <= maximum_length
                && bar.length.is_power_of_two()
                && bar.address != 0
                && bar.address % bar.length == 0
                && end
                    <= <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::traits::MemoryArchitecture>::physical_address_limit()
                && !memory_regions.iter().any(|region| {
                    region
                        .start
                        .checked_add(region.length)
                        .is_some_and(|region_end| {
                            region.start < end as usize && (bar.address as usize) < region_end
                        })
                })
            {
                return Some(bar);
            }
        }
        index += if is_64_bit { 2 } else { 1 };
    }
    None
}

fn probe_memory_bar(address: PciAddress, index: u8) -> Option<(Option<PciBar>, bool)> {
    if index >= 6 {
        return None;
    }
    let _guard = CONFIG_LOCK.lock();
    let offset = 0x10 + index * 4;
    let original_low = read_config_unlocked(address, offset);
    if original_low == 0 || original_low == u32::MAX || original_low & 1 != 0 {
        return Some((None, false));
    }
    let memory_type = (original_low >> 1) & 3;
    let is_64_bit = memory_type == 2;
    if memory_type != 0 && !is_64_bit || is_64_bit && index == 5 {
        return Some((None, is_64_bit));
    }
    let original_high = if is_64_bit {
        Some(read_config_unlocked(address, offset.checked_add(4)?))
    } else {
        None
    };
    let command = read_config_u16_unlocked(address, 0x04);
    let mut restore = BarProbeRestore {
        address,
        offset,
        low: original_low,
        high: original_high,
        command,
        restored: false,
    };
    write_config_u16_unlocked(address, 0x04, command & !(1 << 1));
    if is_64_bit {
        write_config_unlocked(address, offset.checked_add(4)?, u32::MAX);
    }
    write_config_unlocked(address, offset, u32::MAX);
    let mask_low = read_config_unlocked(address, offset);
    let mask_high = if is_64_bit {
        Some(read_config_unlocked(address, offset.checked_add(4)?))
    } else {
        None
    };
    restore.restore()?;

    let address_mask = if let Some(mask_high) = mask_high {
        (u64::from(mask_high) << 32) | u64::from(mask_low & !0xf)
    } else {
        u64::from((!(mask_low & !0xf)).wrapping_add(1))
    };
    let length = if is_64_bit {
        (!address_mask).wrapping_add(1)
    } else {
        address_mask
    };
    let base = (u64::from(original_low & !0xf))
        | (u64::from(original_high.unwrap_or(0)) << 32);
    if length == 0 {
        return Some((None, is_64_bit));
    }
    Some((Some(PciBar { index, address: base, length }), is_64_bit))
}

struct BarProbeRestore {
    address: PciAddress,
    offset: u8,
    low: u32,
    high: Option<u32>,
    command: u16,
    restored: bool,
}

impl BarProbeRestore {
    fn restore(&mut self) -> Option<()> {
        if let Some(high) = self.high {
            write_config_unlocked(self.address, self.offset.checked_add(4)?, high);
        }
        write_config_unlocked(self.address, self.offset, self.low);
        write_config_u16_unlocked(self.address, 0x04, self.command);
        self.restored = true;
        Some(())
    }
}

impl Drop for BarProbeRestore {
    fn drop(&mut self) {
        if !self.restored {
            let _ = self.restore();
        }
    }
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
