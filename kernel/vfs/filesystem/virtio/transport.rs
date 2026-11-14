/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::arch::traits::DeviceArchitecture as _;
use crate::kernel::error::Error;
use crate::kernel::memory::dma::DmaArena;

#[repr(usize)]
#[derive(Clone, Copy)]
enum Register {
    Magic = 0x00,
    Version = 0x04,
    DeviceId = 0x08,
    DeviceFeatures = 0x10,
    DeviceFeaturesSelect = 0x14,
    DriverFeatures = 0x20,
    DriverFeaturesSelect = 0x24,
    QueueSelect = 0x30,
    QueueSizeMax = 0x34,
    QueueSize = 0x38,
    QueueReady = 0x44,
    QueueNotify = 0x50,
    InterruptStatus = 0x60,
    InterruptAck = 0x64,
    Status = 0x70,
}

#[repr(usize)]
#[derive(Clone, Copy)]
enum AddressRegister {
    QueueDesc = 0x80,
    QueueAvail = 0x90,
    QueueUsed = 0xa0,
}

const REGISTER_WINDOW_SIZE: usize =
    AddressRegister::QueueUsed as usize + core::mem::size_of::<u64>();

#[derive(Clone, Copy)]
struct MappedMmioWindow {
    start: usize,
    slots: usize,
    stride: usize,
}

impl MappedMmioWindow {
    fn from_arch() -> Option<Self> {
        if !crate::kernel::memory::r#virtual::mmu_ready() {
            return None;
        }
        let window = crate::kernel::arch::DeviceArchitecture::VIRTIO_WINDOW;
        let register_size = core::mem::size_of::<u32>();
        if window.slots == 0
            || window.stride < REGISTER_WINDOW_SIZE
            || window.stride % register_size != 0
            || window.start % register_size != 0
        {
            return None;
        }
        let length = window.stride.checked_mul(window.slots)?;
        let _window_end = window.start.checked_add(length)?;
        Some(Self {
            start: window.start,
            slots: window.slots,
            stride: window.stride,
        })
    }

    fn region(&self, slot: usize) -> Option<MmioRegion> {
        if slot >= self.slots {
            return None;
        }
        let base = self.start.checked_add(slot.checked_mul(self.stride)?)?;
        base.checked_add(self.stride)?;
        Some(MmioRegion {
            base,
            stride: self.stride,
            slot,
        })
    }

    fn region_at_base(&self, base: usize) -> Option<MmioRegion> {
        let offset = base.checked_sub(self.start)?;
        if offset % self.stride != 0 {
            return None;
        }
        let region = self.region(offset / self.stride)?;
        (region.base == base).then_some(region)
    }
}

#[derive(Clone, Copy, Debug)]
struct MmioRegion {
    base: usize,
    stride: usize,
    slot: usize,
}

impl MmioRegion {
    fn checked_address(&self, offset: usize) -> Option<usize> {
        let register_size = core::mem::size_of::<u32>();
        if offset % register_size != 0 || offset.checked_add(register_size)? > self.stride {
            return None;
        }
        self.base.checked_add(offset)
    }

    fn read_u32(&self, offset: usize) -> Option<u32> {
        let address = self.checked_address(offset)?;
        // SAFETY: regions come from the architecture's mapped VirtIO window, and the checked
        // offset is aligned and contained within the region.
        Some(unsafe { core::ptr::read_volatile(address as *const u32) })
    }

    fn write_u32(&self, offset: usize, value: u32) -> Option<()> {
        let address = self.checked_address(offset)?;
        // SAFETY: regions come from the architecture's mapped VirtIO window, and the checked
        // offset is aligned and contained within the region.
        unsafe { core::ptr::write_volatile(address as *mut u32, value) };
        Some(())
    }

    fn read_u8(&self, offset: usize) -> Option<u8> {
        if offset >= self.stride {
            return None;
        }
        let address = self.base.checked_add(offset)?;
        // SAFETY: regions come from the architecture's mapped VirtIO window, and the checked
        // offset is contained within the region.
        Some(unsafe { core::ptr::read_volatile(address as *const u8) })
    }

    #[allow(dead_code)]
    fn write_u8(&self, offset: usize, value: u8) -> Option<()> {
        if offset >= self.stride {
            return None;
        }
        let address = self.base.checked_add(offset)?;
        // SAFETY: regions come from the architecture's mapped VirtIO window, and the checked
        // offset is contained within the region.
        unsafe { core::ptr::write_volatile(address as *mut u8, value) };
        Some(())
    }

    fn read_register(&self, register: Register) -> u32 {
        self.read_u32(register as usize)
            .expect("typed VirtIO register must fit the region")
    }

    fn write_register(&self, register: Register, value: u32) {
        self.write_u32(register as usize, value)
            .expect("typed VirtIO register must fit the region");
    }

    fn write_address(&self, register: AddressRegister, address: u64) {
        let offset = register as usize;
        self.write_u32(offset, address as u32)
            .expect("typed VirtIO address register must fit the region");
        self.write_u32(offset + core::mem::size_of::<u32>(), (address >> 32) as u32)
            .expect("typed VirtIO address register must fit the region");
    }

    fn read_config(&self, offset: usize) -> Option<u32> {
        self.read_u32(0x100usize.checked_add(offset)?)
    }

    #[allow(dead_code)]
    fn write_config(&self, offset: usize, value: u32) -> Option<()> {
        self.write_u32(0x100usize.checked_add(offset)?, value)
    }

    fn read_config_u8(&self, offset: usize) -> Option<u8> {
        self.read_u8(0x100usize.checked_add(offset)?)
    }

    #[allow(dead_code)]
    fn write_config_u8(&self, offset: usize, value: u8) -> Option<()> {
        self.write_u8(0x100usize.checked_add(offset)?, value)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct VirtioTransport {
    region: MmioRegion,
}

impl VirtioTransport {
    const MAGIC: u32 = 0x7472_6976;
    const VERSION_MODERN: u32 = 2;
    const FEATURE_VERSION_1: u32 = 1;
    const FEATURE_NET_MTU: u64 = 1 << 3;
    const FEATURE_NET_MAC: u64 = 1 << 5;

    pub(crate) fn from_base(base: usize) -> Option<Self> {
        let window = MappedMmioWindow::from_arch()?;
        Some(Self {
            region: window.region_at_base(base)?,
        })
    }

    pub(crate) fn find(device_id: u32) -> Option<Self> {
        let window = MappedMmioWindow::from_arch()?;
        for slot in 0..window.slots {
            let region = window.region(slot)?;
            let magic = region.read_register(Register::Magic);
            let version = region.read_register(Register::Version);
            let id = region.read_register(Register::DeviceId);
            if magic == Self::MAGIC && version == Self::VERSION_MODERN && id == device_id {
                return Some(Self { region });
            }
        }
        None
    }

    pub(crate) const fn device_type_name(device_id: u32) -> &'static str {
        match device_id {
            1 => "network",
            2 => "block",
            3 => "console",
            4 => "entropy",
            5 => "balloon",
            6 => "iomem",
            7 => "rpmsg",
            8 => "scsi",
            9 => "9p",
            10 => "mac80211-wlan",
            11 => "rproc-serial",
            12 => "caif",
            13 => "memory-balloon",
            16 => "gpu",
            17 => "clock",
            18 => "input",
            19 => "vsock",
            20 => "crypto",
            21 => "signal-dist",
            22 => "pstore",
            23 => "iommu",
            24 => "memory",
            25 => "sound",
            26 => "filesystem",
            27 => "pmem",
            28 => "rpmb",
            29 => "mac80211-hwsim",
            30 => "video-encoder",
            31 => "video-decoder",
            32 => "scmi",
            33 => "nitro-secure-module",
            34 => "i2c-adapter",
            35 => "watchdog",
            36 => "can",
            37 => "dmabuf",
            38 => "parameter-server",
            39 => "audio-policy",
            40 => "bluetooth",
            41 => "gpio",
            42 => "rtc",
            43 => "tpm",
            _ => "unknown",
        }
    }

    pub(crate) fn initialize(self, device_id: u32) -> Result<Self, Error> {
        self.initialize_with_features(device_id, 0)
    }

    pub(crate) fn initialize_with_features(
        self,
        device_id: u32,
        requested_features: u64,
    ) -> Result<Self, Error> {
        self.initialize_with_optional_features(device_id, requested_features, 0)
            .map(|(transport, _)| transport)
    }

    pub(crate) fn initialize_with_optional_features(
        self,
        device_id: u32,
        required_features: u64,
        optional_features: u64,
    ) -> Result<(Self, u64), Error> {
        let region = self.region;
        if region.read_register(Register::Magic) != Self::MAGIC
            || region.read_register(Register::Version) != Self::VERSION_MODERN
            || region.read_register(Register::DeviceId) != device_id
        {
            return Err(Error::NoDev);
        }

        region.write_register(Register::Status, 0);
        region.write_register(Register::Status, 1);
        region.write_register(Register::Status, 3);

        region.write_register(Register::DeviceFeaturesSelect, 0);
        let offered_low = region.read_register(Register::DeviceFeatures);
        region.write_register(Register::DeviceFeaturesSelect, 1);
        let offered_high = region.read_register(Register::DeviceFeatures);
        if offered_high & Self::FEATURE_VERSION_1 == 0 {
            region.write_register(Register::Status, 0x80);
            return Err(Error::OpNotSupp);
        }
        let offered_features = u64::from(offered_low) | (u64::from(offered_high) << 32);
        if required_features & !offered_features != 0 {
            region.write_register(Register::Status, 0x80);
            return Err(Error::OpNotSupp);
        }
        let negotiated_features = required_features | (optional_features & offered_features);
        let requested_low = negotiated_features as u32;
        let requested_high = (negotiated_features >> 32) as u32 | Self::FEATURE_VERSION_1;
        region.write_register(Register::DriverFeaturesSelect, 0);
        region.write_register(Register::DriverFeatures, requested_low);
        region.write_register(Register::DriverFeaturesSelect, 1);
        region.write_register(Register::DriverFeatures, requested_high);
        region.write_register(Register::Status, 11);
        if region.read_register(Register::Status) & 8 == 0 {
            region.write_register(Register::Status, 0x80);
            return Err(Error::Proto);
        }
        Ok((self, negotiated_features))
    }

    pub(crate) fn configure_queue(
        &self,
        index: u32,
        maximum_size: u32,
        minimum_size: u32,
        descriptors: u64,
        available: u64,
        used: u64,
    ) -> Result<u32, Error> {
        self.region.write_register(Register::QueueSelect, index);
        if self.region.read_register(Register::QueueReady) != 0 {
            return Err(Error::Busy);
        }
        let maximum_size = self
            .region
            .read_register(Register::QueueSizeMax)
            .min(maximum_size);
        if maximum_size == 0 || minimum_size == 0 {
            return Err(Error::Inval);
        }
        let size = 1u32 << (31 - maximum_size.leading_zeros());
        if size < minimum_size {
            return Err(Error::Inval);
        }
        self.region.write_register(Register::QueueSize, size);
        self.region
            .write_address(AddressRegister::QueueDesc, descriptors);
        self.region
            .write_address(AddressRegister::QueueAvail, available);
        self.region.write_address(AddressRegister::QueueUsed, used);
        DmaArena::sync_for_device();
        self.region.write_register(Register::QueueReady, 1);
        Ok(size)
    }

    pub(crate) fn activate(&self) -> bool {
        self.region.write_register(Register::Status, 15);
        self.region.read_register(Register::Status) & 4 != 0
    }

    pub(crate) fn notify(&self, queue: u32) {
        self.region.write_register(Register::QueueNotify, queue);
    }

    pub(crate) fn read_device_config(&self, offset: usize) -> Option<u32> {
        self.region.read_config(offset)
    }

    pub(crate) fn read_device_config_u16(&self, offset: usize) -> Option<u16> {
        let low = self.region.read_config_u8(offset)?;
        let high = self.region.read_config_u8(offset.checked_add(1)?)?;
        Some(u16::from_le_bytes([low, high]))
    }

    #[allow(dead_code)]
    pub(crate) fn write_device_config(&self, offset: usize, value: u32) -> Option<()> {
        self.region.write_config(offset, value)
    }

    #[allow(dead_code)]
    pub(crate) fn read_device_config_byte(&self, offset: usize) -> Option<u8> {
        self.region.read_config_u8(offset)
    }

    #[allow(dead_code)]
    pub(crate) fn write_device_config_byte(&self, offset: usize, value: u8) -> Option<()> {
        self.region.write_config_u8(offset, value)
    }

    pub(crate) fn slot(&self) -> usize {
        self.region.slot
    }

    pub(crate) fn base(&self) -> usize {
        self.region.base
    }

    pub(crate) const fn net_mac_feature() -> u64 {
        Self::FEATURE_NET_MAC
    }

    pub(crate) const fn net_mtu_feature() -> u64 {
        Self::FEATURE_NET_MTU
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VirtioDeviceInfo {
    pub(crate) device_id: u32,
    pub(crate) slot: usize,
}

pub(crate) fn for_each_device(mut callback: impl FnMut(VirtioDeviceInfo)) {
    let Some(window) = MappedMmioWindow::from_arch() else {
        return;
    };
    for slot in 0..window.slots {
        let Some(region) = window.region(slot) else {
            break;
        };
        let magic = region.read_register(Register::Magic);
        let version = region.read_register(Register::Version);
        let device_id = region.read_register(Register::DeviceId);
        if magic == VirtioTransport::MAGIC
            && version == VirtioTransport::VERSION_MODERN
            && device_id != 0
        {
            callback(VirtioDeviceInfo {
                device_id,
                slot: region.slot,
            });
        }
    }
}

pub(crate) fn dispatch_interrupts() -> u64 {
    if !crate::kernel::memory::r#virtual::mmu_ready() {
        return 0;
    }
    let Some(window) = MappedMmioWindow::from_arch() else {
        return 0;
    };
    let mut pending = 0u64;
    for slot in 0..window.slots.min(64) {
        let Some(region) = window.region(slot) else {
            break;
        };
        let magic = region.read_register(Register::Magic);
        let version = region.read_register(Register::Version);
        let device_id = region.read_register(Register::DeviceId);
        if magic != VirtioTransport::MAGIC
            || version != VirtioTransport::VERSION_MODERN
            || device_id == 0
        {
            continue;
        }
        let status = region.read_register(Register::InterruptStatus);
        if status != 0 {
            region.write_register(Register::InterruptAck, status);
            pending |= 1u64 << slot;
        }
    }
    pending
}
