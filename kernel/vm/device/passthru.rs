/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::arch::traits::VirtualizationArchitecture as _;

const MAX_ASSIGNED_FUNCTIONS: usize = 64;

static ASSIGNED_FUNCTIONS: IrqSpinLock<[Option<PciFunction>; MAX_ASSIGNED_FUNCTIONS]> =
    IrqSpinLock::new([None; MAX_ASSIGNED_FUNCTIONS]);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PciFunction {
    pub(crate) segment: u16,
    pub(crate) bus: u8,
    pub(crate) device: u8,
    pub(crate) function: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ClaimError {
    InvalidAddress,
    AlreadyAssigned,
    RegistryFull,
}

pub(crate) struct ExclusivePciClaim {
    address: PciFunction,
    retained: bool,
}

impl ExclusivePciClaim {
    pub(crate) fn acquire(address: PciFunction) -> Result<Self, ClaimError> {
        if address.device >= 32 || address.function >= 8 {
            return Err(ClaimError::InvalidAddress);
        }
        let mut assigned = ASSIGNED_FUNCTIONS.lock();
        if assigned.contains(&Some(address)) {
            return Err(ClaimError::AlreadyAssigned);
        }
        let Some(slot) = assigned.iter_mut().find(|entry| entry.is_none()) else {
            return Err(ClaimError::RegistryFull);
        };
        *slot = Some(address);
        Ok(Self {
            address,
            retained: false,
        })
    }

    pub(crate) const fn address(&self) -> PciFunction {
        self.address
    }

    pub(crate) fn retain(&mut self) {
        self.retained = true;
    }
}

impl Drop for ExclusivePciClaim {
    fn drop(&mut self) {
        if self.retained {
            return;
        }
        let mut assigned = ASSIGNED_FUNCTIONS.lock();
        if let Some(slot) = assigned.iter_mut().find(|entry| **entry == Some(self.address)) {
            *slot = None;
        }
    }
}

pub(crate) const GUEST_INFO_ADDRESS: u64 = 0x6000;
pub(crate) const GUEST_MMIO_APERTURE: u64 = 0x20_0000;
const GUEST_MMIO_MAX_LENGTH: u64 = 2 * 1024 * 1024;

pub(crate) struct PassthroughDevice {
    guest_mmio_address: u64,
    guest_mmio_host_address: u64,
    guest_mmio_length: usize,
    #[cfg(all(target_arch = "x86_64", feature = "intel-vtd"))]
    _dma: crate::kernel::arch::x86_64::vtd::VtdDmaAssignment,
    _claim: ExclusivePciClaim,
}

impl PassthroughDevice {
    pub(crate) fn attach(
        context: &mut <crate::kernel::arch::VirtualizationArchitecture as crate::kernel::arch::traits::VirtualizationArchitecture>::GuestContext,
        function: PciFunction,
    ) -> Result<Self, crate::kernel::arch::VirtualizationError> {
        let claim = ExclusivePciClaim::acquire(function)
            .map_err(|_| crate::kernel::arch::VirtualizationError::Unsupported)?;

        #[cfg(all(target_arch = "x86_64", feature = "intel-vtd"))]
        {
            use crate::kernel::arch::x86_64::pci::{self, PciAddress};

            let address = PciAddress {
                segment: function.segment,
                bus: function.bus,
                device: function.device,
                function: function.function,
            };
            let prepared = crate::kernel::arch::x86_64::vtd::prepare_for_assignment(address)
                .ok_or(crate::kernel::arch::VirtualizationError::Unsupported)?;
            let bar = pci::find_guest_mmio_bar(address, GUEST_MMIO_MAX_LENGTH)
                .ok_or(crate::kernel::arch::VirtualizationError::Unsupported)?;
            let guest_mmio_length = usize::try_from(bar.length)
                .map_err(|_| crate::kernel::arch::VirtualizationError::Unsupported)?;
            let (guest_memory, guest_memory_length) =
                crate::kernel::arch::VirtualizationArchitecture::guest_memory_region(context)
                    .ok_or(crate::kernel::arch::VirtualizationError::Unsupported)?;
            let mut dma = crate::kernel::arch::x86_64::vtd::VtdDmaAssignment::attach(
                prepared,
                guest_memory,
                guest_memory_length,
            )?;
            let setup_result = (|| {
                crate::kernel::arch::VirtualizationArchitecture::map_guest_mmio(
                    context,
                    GUEST_MMIO_APERTURE,
                    bar.address,
                    guest_mmio_length,
                )?;

                let identity = pci::read_config(address, 0)
                    .ok_or(crate::kernel::arch::VirtualizationError::Unsupported)?;
                let class = pci::read_config(address, 0x08)
                    .ok_or(crate::kernel::arch::VirtualizationError::Unsupported)?;
                let info = GuestPciInfo {
                    segment: function.segment,
                    bus: function.bus,
                    device: function.device,
                    function: function.function,
                    bar_index: bar.index,
                    vendor_id: identity as u16,
                    device_id: (identity >> 16) as u16,
                    class_code: (class >> 24) as u8,
                    subclass: (class >> 16) as u8,
                    programming_interface: (class >> 8) as u8,
                    guest_mmio_address: GUEST_MMIO_APERTURE,
                    mmio_length: bar.length,
                };
                crate::kernel::arch::VirtualizationArchitecture::write_guest_memory(
                    context,
                    GUEST_INFO_ADDRESS,
                    &info.encode(),
                )?;
                dma.activate()
            })();
            if let Err(error) = setup_result {
                if !dma.detach() {
                    claim.retain();
                    crate::kernel::arch::VirtualizationArchitecture::retain_guest_memory(context);
                }
                return Err(error);
            }
            Ok(Self {
                guest_mmio_address: GUEST_MMIO_APERTURE,
                guest_mmio_host_address: bar.address,
                guest_mmio_length,
                _dma: dma,
                _claim: claim,
            })
        }

        #[cfg(not(all(target_arch = "x86_64", feature = "intel-vtd")))]
        {
            let _ = (context, function, claim);
            Err(crate::kernel::arch::VirtualizationError::Unsupported)
        }
    }

    pub(crate) fn detach(&mut self) -> bool {
        #[cfg(all(target_arch = "x86_64", feature = "intel-vtd"))]
        {
            let detached = if self.is_missing() {
                self._dma.detach_after_removal()
            } else {
                self._dma.detach()
            };
            if !detached {
                self._claim.retain();
                return false;
            }
        }
        true
    }

    #[cfg(all(target_arch = "x86_64", feature = "intel-vtd"))]
    pub(crate) fn is_missing(&self) -> bool {
        use crate::kernel::arch::x86_64::pci::{self, PciAddress};

        let function = self._claim.address();
        pci::function_is_present(PciAddress {
            segment: function.segment,
            bus: function.bus,
            device: function.device,
            function: function.function,
        }) == Some(false)
    }

    #[cfg(not(all(target_arch = "x86_64", feature = "intel-vtd")))]
    pub(crate) fn is_missing(&self) -> bool {
        false
    }

    pub(crate) const fn guest_mmio_mapping(&self) -> (u64, u64, usize) {
        (
            self.guest_mmio_address,
            self.guest_mmio_host_address,
            self.guest_mmio_length,
        )
    }
}

impl Drop for PassthroughDevice {
    fn drop(&mut self) {
        let _detached = self.detach();
    }
}

struct GuestPciInfo {
    segment: u16,
    bus: u8,
    device: u8,
    function: u8,
    bar_index: u8,
    vendor_id: u16,
    device_id: u16,
    class_code: u8,
    subclass: u8,
    programming_interface: u8,
    guest_mmio_address: u64,
    mmio_length: u64,
}

impl GuestPciInfo {
    fn encode(&self) -> [u8; 36] {
        let mut bytes = [0; 36];
        bytes[0..4].copy_from_slice(b"KZPC");
        bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&self.segment.to_le_bytes());
        bytes[8] = self.bus;
        bytes[9] = self.device;
        bytes[10] = self.function;
        bytes[11] = self.bar_index;
        bytes[12..14].copy_from_slice(&self.vendor_id.to_le_bytes());
        bytes[14..16].copy_from_slice(&self.device_id.to_le_bytes());
        bytes[16] = self.class_code;
        bytes[17] = self.subclass;
        bytes[18] = self.programming_interface;
        bytes[20..28].copy_from_slice(&self.guest_mmio_address.to_le_bytes());
        bytes[28..36].copy_from_slice(&self.mmio_length.to_le_bytes());
        bytes
    }
}