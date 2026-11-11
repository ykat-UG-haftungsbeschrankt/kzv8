/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::mem::forget;
use core::ptr::{write_bytes, write_volatile};
use core::sync::atomic::{fence, Ordering};

use crate::kernel::arch::traits::MemoryArchitecture as _;
use crate::kernel::arch::{MemoryArchitecture, VirtualizationError};
use crate::kernel::memory::physical::PhysicalAllocator;
use crate::kernel::scheduler::sync::IrqSpinLock;

const PAGE_SIZE: usize = 4096;
const GUEST_MEMORY_SIZE: usize = 2 * 1024 * 1024;
const DMA_READ: u64 = 1 << 0;
const DMA_WRITE: u64 = 1 << 1;
const DMA_LARGE_PAGE: u64 = 1 << 7;
const ROOT_TABLE_REGISTER: usize = 0x20;
const GLOBAL_COMMAND_REGISTER: usize = 0x18;
const GLOBAL_STATUS_REGISTER: usize = 0x1c;
const CONTEXT_COMMAND_REGISTER: usize = 0x28;
const ROOT_TABLE_POINTER_SET: u32 = 1 << 30;
const TRANSLATION_ENABLE: u32 = 1 << 31;
const ROOT_TABLE_POINTER_STATUS: u32 = 1 << 30;
const TRANSLATION_STATUS: u32 = 1 << 31;
const WRITE_BUFFER_FLUSH: u32 = 1 << 27;
const WRITE_BUFFER_FLUSH_STATUS: u32 = 1 << 27;
const CONTEXT_INVALIDATE: u64 = 1 << 63;
const GLOBAL_CONTEXT_INVALIDATE: u64 = 1 << 61;
const IOTLB_INVALIDATE: u64 = 1 << 63;
const GLOBAL_IOTLB_INVALIDATE: u64 = 1 << 60;
const REGISTER_WAIT_LIMIT: usize = 100_000;

static VTD_MANAGER: IrqSpinLock<Option<VtdManager>> = IrqSpinLock::new(None);
const MAX_ACTIVE_ASSIGNMENTS: usize = 64;

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

struct VtdPage {
    physical: usize,
}

impl VtdPage {
    fn allocate() -> Result<Self, VirtualizationError> {
        let physical = PhysicalAllocator::allocate_contiguous(PAGE_SIZE, PAGE_SIZE)
            .ok_or(VirtualizationError::ResourceExhausted)?;
        let Some(end) = physical.checked_add(PAGE_SIZE) else {
            let _released = PhysicalAllocator::deallocate_contiguous(physical, PAGE_SIZE);
            return Err(VirtualizationError::HostSetupFailed);
        };
        if physical % PAGE_SIZE != 0
            || end as u64 > MemoryArchitecture::physical_address_limit()
            || !MemoryArchitecture::is_identity_mapped(physical)
            || !MemoryArchitecture::is_identity_mapped(end - 1)
        {
            let _released = PhysicalAllocator::deallocate_contiguous(physical, PAGE_SIZE);
            return Err(VirtualizationError::Unsupported);
        }
        // SAFETY: the allocator returned a writable, identity-mapped page for this table.
        unsafe { write_bytes(physical as *mut u8, 0, PAGE_SIZE) };
        Ok(Self { physical })
    }

    fn write_u64(&self, offset: usize, value: u64) -> bool {
        if offset & 7 != 0 || offset.checked_add(8).is_none_or(|end| end > PAGE_SIZE) {
            return false;
        }
        // SAFETY: this page is an allocator-owned identity-mapped page and the offset is bounded.
        unsafe { write_volatile((self.physical + offset) as *mut u64, value) };
        true
    }
}

impl Drop for VtdPage {
    fn drop(&mut self) {
        let _released = PhysicalAllocator::deallocate_contiguous(self.physical, PAGE_SIZE);
    }
}

struct ContextTable {
    bus: u8,
    page: VtdPage,
}

struct ActiveContext {
    address: super::pci::PciAddress,
    domain_id: u16,
}

struct VtdManager {
    register_base: usize,
    capability: u64,
    extended_capability: u64,
    root_table: VtdPage,
    context_tables: Vec<ContextTable>,
    active_contexts: Vec<ActiveContext>,
    next_domain_id: u32,
    poisoned: bool,
}

impl VtdManager {
    fn create(
        address: super::pci::PciAddress,
        guest_memory: usize,
    ) -> Result<(Self, VtdPage, u16), VirtualizationError> {
        let dmar = super::acpi::dmar_info().ok_or(VirtualizationError::Unsupported)?;
        if dmar.units.len() != 1
            || !dmar.units[0].include_all
            || dmar.units[0].segment != 0
            || !dmar.reserved_regions.is_empty()
            || address.segment != 0
            || address.bus != 0
            || guest_memory % GUEST_MEMORY_SIZE != 0
        {
            return Err(VirtualizationError::Unsupported);
        }
        if super::acpi::mcfg_allocations()
            .is_some_and(|allocations| allocations.iter().any(|allocation| allocation.segment != 0))
        {
            return Err(VirtualizationError::Unsupported);
        }

        let register_base = usize::try_from(dmar.units[0].register_base)
            .map_err(|_| VirtualizationError::Unsupported)?;
        let capability_address = register_base
            .checked_add(8)
            .ok_or(VirtualizationError::Unsupported)?;
        let capability = super::paging::read_mmio_u64(capability_address)
            .ok_or(VirtualizationError::Unsupported)?;
        let extended_capability = super::paging::read_mmio_u64(
            register_base
                .checked_add(16)
                .ok_or(VirtualizationError::Unsupported)?,
        )
        .ok_or(VirtualizationError::Unsupported)?;
        if (capability >> 8) & 1 == 0
            || capability & (1 << 34) == 0
            || extended_capability & 1 == 0
            || ((extended_capability >> 8) & 0x3ff) == 0
        {
            return Err(VirtualizationError::Unsupported);
        }
        let status_address = register_base
            .checked_add(GLOBAL_STATUS_REGISTER)
            .ok_or(VirtualizationError::Unsupported)?;
        let status = super::paging::read_mmio_u32(status_address)
            .ok_or(VirtualizationError::Unsupported)?;
        if status & (TRANSLATION_STATUS | ROOT_TABLE_POINTER_STATUS) != 0 {
            return Err(VirtualizationError::Unsupported);
        }

        let root_table = VtdPage::allocate()?;
        let context_page = VtdPage::allocate()?;
        let second_level = VtdPage::allocate()?;
        let host_address_width = ((capability >> 16) & 0x3f) + 1;
        let address_limit = 1u128 << host_address_width;
        if guest_memory as u64 > MemoryArchitecture::physical_address_limit()
            || guest_memory
                .checked_add(GUEST_MEMORY_SIZE)
                .is_none_or(|end| end as u64 > MemoryArchitecture::physical_address_limit())
            || guest_memory as u128 + GUEST_MEMORY_SIZE as u128 > address_limit
            || !MemoryArchitecture::is_identity_mapped(guest_memory)
            || !MemoryArchitecture::is_identity_mapped(guest_memory + GUEST_MEMORY_SIZE - 1)
            || [root_table.physical, context_page.physical, second_level.physical]
                .iter()
                .any(|page| *page as u128 + PAGE_SIZE as u128 > address_limit)
        {
            return Err(VirtualizationError::Unsupported);
        }

        let domain_id = 2;
        let root_offset = usize::from(address.bus) * 16;
        let context_index = usize::from(address.device) * 8 + usize::from(address.function);
        if !second_level.write_u64(0, guest_memory as u64 | DMA_READ | DMA_WRITE | DMA_LARGE_PAGE)
            || !context_page.write_u64(context_index * 16 + 8, u64::from(domain_id) << 8)
        {
            return Err(VirtualizationError::HostSetupFailed);
        }
        fence(Ordering::SeqCst);
        if !context_page.write_u64(context_index * 16, second_level.physical as u64 | 1)
            || !root_table.write_u64(root_offset, context_page.physical as u64 | 1)
        {
            return Err(VirtualizationError::HostSetupFailed);
        }

        let mut context_tables = Vec::new();
        context_tables
            .try_reserve_exact(1)
            .map_err(|_| VirtualizationError::ResourceExhausted)?;
        context_tables.push(ContextTable {
            bus: address.bus,
            page: context_page,
        });
        let mut active_contexts = Vec::new();
        active_contexts
            .try_reserve_exact(1)
            .map_err(|_| VirtualizationError::ResourceExhausted)?;
        active_contexts.push(ActiveContext {
            address,
            domain_id,
        });

        let mut manager = Self {
            register_base,
            capability,
            extended_capability,
            root_table,
            context_tables,
            active_contexts,
            next_domain_id: 3,
            poisoned: false,
        };
        if let Err(error) = manager.program_root_table() {
            forget(manager);
            forget(second_level);
            return Err(error);
        }
        Ok((manager, second_level, domain_id))
    }

    fn attach(
        &mut self,
        address: super::pci::PciAddress,
        guest_memory: usize,
    ) -> Result<(VtdPage, u16), VirtualizationError> {
        if self.poisoned || address.bus != 0 {
            return Err(VirtualizationError::HostSetupFailed);
        }
        if self
            .active_contexts
            .iter()
            .any(|active| active.address == address)
        {
            return Err(VirtualizationError::Unsupported);
        }
        let domain_id = self.allocate_domain_id()?;
        let second_level = VtdPage::allocate()?;
        let host_address_width = ((self.capability >> 16) & 0x3f) + 1;
        let address_limit = 1u128 << host_address_width;
        if guest_memory % GUEST_MEMORY_SIZE != 0
            || guest_memory as u64 > MemoryArchitecture::physical_address_limit()
            || guest_memory
                .checked_add(GUEST_MEMORY_SIZE)
                .is_none_or(|end| end as u64 > MemoryArchitecture::physical_address_limit())
            || guest_memory as u128 + GUEST_MEMORY_SIZE as u128 > address_limit
            || second_level.physical as u128 + PAGE_SIZE as u128 > address_limit
            || !MemoryArchitecture::is_identity_mapped(guest_memory)
            || !MemoryArchitecture::is_identity_mapped(guest_memory + GUEST_MEMORY_SIZE - 1)
            || !second_level.write_u64(
                0,
                guest_memory as u64 | DMA_READ | DMA_WRITE | DMA_LARGE_PAGE,
            )
        {
            return Err(VirtualizationError::Unsupported);
        }

        self.active_contexts
            .try_reserve(1)
            .map_err(|_| VirtualizationError::ResourceExhausted)?;

        let context_index = usize::from(address.device) * 8 + usize::from(address.function);
        let table_index = self
            .context_tables
            .iter()
            .position(|table| table.bus == address.bus);
        let context_page = if let Some(index) = table_index {
            &self.context_tables[index].page
        } else {
            self.context_tables
                .try_reserve(1)
                .map_err(|_| VirtualizationError::ResourceExhausted)?;
            let context = VtdPage::allocate()?;
            let host_address_width = ((self.capability >> 16) & 0x3f) + 1;
            if context.physical as u128 + PAGE_SIZE as u128 > (1u128 << host_address_width) {
                return Err(VirtualizationError::Unsupported);
            }
            if !self
                .root_table
                .write_u64(usize::from(address.bus) * 16, context.physical as u64 | 1)
            {
                return Err(VirtualizationError::HostSetupFailed);
            }
            self.context_tables.push(ContextTable {
                bus: address.bus,
                page: context,
            });
            &self.context_tables.last().ok_or(VirtualizationError::HostSetupFailed)?.page
        };
        if !context_page.write_u64(context_index * 16 + 8, u64::from(domain_id) << 8)
        {
            return Err(VirtualizationError::HostSetupFailed);
        }
        fence(Ordering::SeqCst);
        if !context_page.write_u64(context_index * 16, second_level.physical as u64 | 1) {
            return Err(VirtualizationError::HostSetupFailed);
        }
        fence(Ordering::SeqCst);
        if !self.flush_write_buffer() {
            self.poisoned = true;
            forget(second_level);
            return Err(VirtualizationError::HostSetupFailed);
        }
        if !self.invalidate_all() {
            self.poisoned = true;
            forget(second_level);
            return Err(VirtualizationError::HostSetupFailed);
        }
        self.active_contexts.push(ActiveContext {
            address,
            domain_id,
        });
        Ok((second_level, domain_id))
    }

    fn allocate_domain_id(&mut self) -> Result<u16, VirtualizationError> {
        let supported_ids = 1u32
            .checked_shl(4 + 2 * (self.capability as u32 & 0x7))
            .unwrap_or(u32::MAX)
            .min(u32::from(u16::MAX) + 1);
        if self.next_domain_id >= supported_ids {
            return Err(VirtualizationError::ResourceExhausted);
        }
        let domain_id = self.next_domain_id as u16;
        self.next_domain_id += 1;
        Ok(domain_id)
    }

    fn program_root_table(&self) -> Result<(), VirtualizationError> {
        let root_register = self
            .register_base
            .checked_add(ROOT_TABLE_REGISTER)
            .ok_or(VirtualizationError::Unsupported)?;
        let command_register = self
            .register_base
            .checked_add(GLOBAL_COMMAND_REGISTER)
            .ok_or(VirtualizationError::Unsupported)?;
        let status_register = self
            .register_base
            .checked_add(GLOBAL_STATUS_REGISTER)
            .ok_or(VirtualizationError::Unsupported)?;
        if !self.flush_write_buffer() {
            return Err(VirtualizationError::HostSetupFailed);
        }
        super::paging::write_mmio_u64(root_register, self.root_table.physical as u64)
            .ok_or(VirtualizationError::Unsupported)?;
        let command = super::paging::read_mmio_u32(command_register)
            .ok_or(VirtualizationError::Unsupported)?;
        super::paging::write_mmio_u32(command_register, command | ROOT_TABLE_POINTER_SET)
            .ok_or(VirtualizationError::Unsupported)?;
        if !wait_register32(status_register, ROOT_TABLE_POINTER_STATUS, true)
            || !self.flush_write_buffer()
            || !self.invalidate_all()
        {
            return Err(VirtualizationError::HostSetupFailed);
        }
        super::paging::write_mmio_u32(command_register, command | TRANSLATION_ENABLE)
            .ok_or(VirtualizationError::Unsupported)?;
        if !wait_register32(status_register, TRANSLATION_STATUS, true) {
            return Err(VirtualizationError::HostSetupFailed);
        }
        Ok(())
    }

    fn invalidate_all(&self) -> bool {
        let Some(context_register) = self.register_base.checked_add(CONTEXT_COMMAND_REGISTER) else {
            return false;
        };
        if super::paging::write_mmio_u64(
            context_register,
            CONTEXT_INVALIDATE | GLOBAL_CONTEXT_INVALIDATE,
        )
        .is_none()
            || !wait_register64(context_register, CONTEXT_INVALIDATE, false)
        {
            return false;
        }
        let offset = (((self.extended_capability >> 8) & 0x3ff) as usize) * 16;
        let Some(invalidate_register) = self.register_base.checked_add(offset + 8) else {
            return false;
        };
        if super::paging::write_mmio_u64(
            invalidate_register,
            IOTLB_INVALIDATE | GLOBAL_IOTLB_INVALIDATE,
        )
        .is_none()
            || !wait_register64(invalidate_register, IOTLB_INVALIDATE, false)
        {
            return false;
        }
        super::paging::read_mmio_u64(invalidate_register)
            .is_some_and(|value| (value >> 57) & 0x3 != 0)
    }

    fn flush_write_buffer(&self) -> bool {
        if self.capability & (1 << 4) == 0 {
            return true;
        }
        let Some(command_register) = self.register_base.checked_add(GLOBAL_COMMAND_REGISTER) else {
            return false;
        };
        let Some(status_register) = self.register_base.checked_add(GLOBAL_STATUS_REGISTER) else {
            return false;
        };
        let Some(command) = super::paging::read_mmio_u32(command_register) else {
            return false;
        };
        super::paging::write_mmio_u32(command_register, command | WRITE_BUFFER_FLUSH).is_some()
            && wait_register32(status_register, WRITE_BUFFER_FLUSH_STATUS, false)
    }

    fn remove(&mut self, address: super::pci::PciAddress, domain_id: u16) -> bool {
        if self.poisoned {
            return false;
        }
        let Some(active_index) = self.active_contexts.iter().position(|active| {
            active.address == address && active.domain_id == domain_id
        }) else {
            return false;
        };
        let Some(context_table) = self
            .context_tables
            .iter()
            .find(|table| table.bus == address.bus)
        else {
            return false;
        };
        let context_index = usize::from(address.device) * 8 + usize::from(address.function);
        let low_address = context_table.page.physical + context_index * 16;
        // SAFETY: the context entry lies in an owned 4 KiB aligned table.
        let low = unsafe { core::ptr::read_volatile(low_address as *const u64) };
        if low & 1 == 0 {
            return false;
        }
        // SAFETY: clear Present before invalidation so the hardware cannot use the old domain.
        unsafe { write_volatile(low_address as *mut u64, low & !1) };
        fence(Ordering::SeqCst);
        if !self.flush_write_buffer() || !self.invalidate_all() {
            self.poisoned = true;
            return false;
        }
        // SAFETY: the invalidation completed and the context entry remains owned by this manager.
        unsafe {
            write_volatile(low_address as *mut u64, 0);
            write_volatile((low_address + 8) as *mut u64, 0);
        }
        self.active_contexts.swap_remove(active_index);
        true
    }
}

pub(crate) fn prepare_for_assignment(
    address: super::pci::PciAddress,
) -> Option<super::pci::PreparedPciFunction> {
    let placeholder = super::pci::PciAddress {
        segment: 0,
        bus: 0,
        device: 0,
        function: 0,
    };
    let mut assigned = [placeholder; MAX_ACTIVE_ASSIGNMENTS];
    let assigned_count = {
        let manager = VTD_MANAGER.lock();
        let Some(manager) = manager.as_ref() else {
            return super::pci::prepare_for_assignment(address, &assigned[..0]);
        };
        if manager.poisoned || manager.active_contexts.len() > assigned.len() {
            return None;
        }
        for (slot, active) in assigned.iter_mut().zip(manager.active_contexts.iter()) {
            *slot = active.address;
        }
        manager.active_contexts.len()
    };
    super::pci::prepare_for_assignment(address, &assigned[..assigned_count])
}

pub(crate) struct VtdDmaAssignment {
    prepared: super::pci::PreparedPciFunction,
    domain_id: u16,
    second_level: Option<VtdPage>,
    detach_attempted: bool,
    detached: bool,
}

impl VtdDmaAssignment {
    pub(crate) fn attach(
        prepared: super::pci::PreparedPciFunction,
        guest_memory: usize,
        guest_memory_length: usize,
    ) -> Result<Self, VirtualizationError> {
        let address = prepared.address();
        if guest_memory_length != GUEST_MEMORY_SIZE
            || address.device >= 32
            || address.function >= 8
            || address.segment != 0
            || address.bus != 0
            || super::pci::read_config(address, 0).is_none_or(|identity| {
                identity as u16 == 0 || identity as u16 == u16::MAX
            })
            || super::pci::read_config_u16(address, 0x04)
                .is_none_or(|command| command & (1 << 2) != 0)
        {
            return Err(VirtualizationError::Unsupported);
        }

        let (second_level, domain_id) = {
            let mut manager = VTD_MANAGER.lock();
            if let Some(manager) = manager.as_mut() {
                manager.attach(address, guest_memory)?
            } else {
                let (new_manager, second_level, domain_id) =
                    VtdManager::create(address, guest_memory)?;
                *manager = Some(new_manager);
                (second_level, domain_id)
            }
        };
        Ok(Self {
            prepared,
            domain_id,
            second_level: Some(second_level),
            detach_attempted: false,
            detached: false,
        })
    }

    pub(crate) fn activate(&mut self) -> Result<(), VirtualizationError> {
        super::pci::activate_memory_and_bus_mastering(self.prepared.address())
            .ok_or(VirtualizationError::HostSetupFailed)
    }

    pub(crate) fn detach(&mut self) -> bool {
        if self.detach_attempted {
            return self.detached;
        }
        self.detach_attempted = true;
        let address = self.prepared.address();
        if super::pci::quiesce_for_detach(address).is_none() {
            if let Some(page) = self.second_level.take() {
                forget(page);
            }
            return false;
        }
        let removed = VTD_MANAGER
            .lock()
            .as_mut()
            .is_some_and(|manager| manager.remove(address, self.domain_id));
        if removed {
            self.second_level.take();
            self.detached = true;
        } else if let Some(page) = self.second_level.take() {
            forget(page);
        }
        self.detached
    }
}

impl Drop for VtdDmaAssignment {
    fn drop(&mut self) {
        let _detached = self.detach();
    }
}

fn wait_register32(address: usize, mask: u32, expected_set: bool) -> bool {
    for _ in 0..REGISTER_WAIT_LIMIT {
        let Some(value) = super::paging::read_mmio_u32(address) else {
            return false;
        };
        if (value & mask != 0) == expected_set {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn wait_register64(address: usize, mask: u64, expected_set: bool) -> bool {
    for _ in 0..REGISTER_WAIT_LIMIT {
        let Some(value) = super::paging::read_mmio_u64(address) else {
            return false;
        };
        if (value & mask != 0) == expected_set {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}