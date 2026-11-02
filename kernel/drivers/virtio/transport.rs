/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

const MAGIC: u32 = 0x7472_6976;
const VERSION_MODERN: u32 = 2;
const FEATURE_VERSION_1: u32 = 1;

const REG_MAGIC: usize = 0x00;
const REG_VERSION: usize = 0x04;
const REG_DEVICE_ID: usize = 0x08;
const REG_DEVICE_FEATURES: usize = 0x10;
const REG_DEVICE_FEATURES_SELECT: usize = 0x14;
const REG_DRIVER_FEATURES: usize = 0x20;
const REG_DRIVER_FEATURES_SELECT: usize = 0x24;
const REG_QUEUE_SELECT: usize = 0x30;
const REG_QUEUE_SIZE_MAX: usize = 0x34;
const REG_QUEUE_SIZE: usize = 0x38;
const REG_QUEUE_READY: usize = 0x44;
const REG_QUEUE_NOTIFY: usize = 0x50;
const REG_INTERRUPT_STATUS: usize = 0x60;
const REG_INTERRUPT_ACK: usize = 0x64;
const REG_STATUS: usize = 0x70;
const REG_QUEUE_DESC: usize = 0x80;
const REG_QUEUE_AVAIL: usize = 0x90;
const REG_QUEUE_USED: usize = 0xa0;
const FEATURE_NET_MAC: u64 = 1 << 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Error {
    InvalidDevice,
    UnsupportedVersion,
    MissingVersionFeature,
    UnsupportedFeature,
    FeatureNegotiation,
    QueueUnavailable,
    InvalidQueueSize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct VirtioTransport {
    base: usize,
}

impl VirtioTransport {
    pub(crate) const fn from_base(base: usize) -> Self {
        Self { base }
    }

    pub(crate) unsafe fn find(device_id: u32) -> Option<Self> {
        let window = crate::arch::virtio_window();
        for slot in 0..window.slots {
            let base = window.start + slot * window.stride;
            let magic = unsafe { read_register(base, REG_MAGIC) };
            let version = unsafe { read_register(base, REG_VERSION) };
            let id = unsafe { read_register(base, REG_DEVICE_ID) };
            if magic == MAGIC && version == VERSION_MODERN && id == device_id {
                return Some(Self::from_base(base));
            }
        }
        None
    }

    pub(crate) unsafe fn initialize(base: usize, device_id: u32) -> Result<Self, Error> {
        Self::initialize_with_features(base, device_id, 0)
    }

    pub(crate) unsafe fn initialize_with_features(
        base: usize,
        device_id: u32,
        requested_features: u64,
    ) -> Result<Self, Error> {
        if read_register(base, REG_MAGIC) != MAGIC
            || read_register(base, REG_VERSION) != VERSION_MODERN
            || read_register(base, REG_DEVICE_ID) != device_id
        {
            return Err(Error::InvalidDevice);
        }

        write_register(base, REG_STATUS, 0);
        write_register(base, REG_STATUS, 1);
        write_register(base, REG_STATUS, 3);

        write_register(base, REG_DEVICE_FEATURES_SELECT, 0);
        let offered_low = read_register(base, REG_DEVICE_FEATURES);
        write_register(base, REG_DEVICE_FEATURES_SELECT, 1);
        let offered_high = read_register(base, REG_DEVICE_FEATURES);
        if offered_high & FEATURE_VERSION_1 == 0 {
            write_register(base, REG_STATUS, 0x80);
            return Err(Error::MissingVersionFeature);
        }
        let requested_low = requested_features as u32;
        let requested_high = (requested_features >> 32) as u32 | FEATURE_VERSION_1;
        if requested_low & !offered_low != 0 || requested_high & !offered_high != 0 {
            write_register(base, REG_STATUS, 0x80);
            return Err(Error::UnsupportedFeature);
        }
        write_register(base, REG_DRIVER_FEATURES_SELECT, 0);
        write_register(base, REG_DRIVER_FEATURES, requested_low);
        write_register(base, REG_DRIVER_FEATURES_SELECT, 1);
        write_register(base, REG_DRIVER_FEATURES, requested_high);
        write_register(base, REG_STATUS, 11);
        if read_register(base, REG_STATUS) & 8 == 0 {
            write_register(base, REG_STATUS, 0x80);
            return Err(Error::FeatureNegotiation);
        }
        Ok(Self { base })
    }

    pub(crate) fn configure_queue(
        &self,
        index: u32,
        size: u32,
        descriptors: u64,
        available: u64,
        used: u64,
    ) -> Result<(), Error> {
        unsafe {
            write_register(self.base, REG_QUEUE_SELECT, index);
            if read_register(self.base, REG_QUEUE_READY) != 0 {
                return Err(Error::QueueUnavailable);
            }
            if read_register(self.base, REG_QUEUE_SIZE_MAX) < size {
                return Err(Error::InvalidQueueSize);
            }
            write_register(self.base, REG_QUEUE_SIZE, size);
            write_address(self.base, REG_QUEUE_DESC, descriptors);
            write_address(self.base, REG_QUEUE_AVAIL, available);
            write_address(self.base, REG_QUEUE_USED, used);
            crate::dma::sync_for_device();
            write_register(self.base, REG_QUEUE_READY, 1);
        }
        Ok(())
    }

    pub(crate) fn activate(&self) -> bool {
        unsafe {
            write_register(self.base, REG_STATUS, 15);
            read_register(self.base, REG_STATUS) & 4 != 0
        }
    }

    pub(crate) fn notify(&self, queue: u32) {
        unsafe {
            write_register(self.base, REG_QUEUE_NOTIFY, queue);
        }
    }

    pub(crate) fn read_device_config(&self, offset: usize) -> u32 {
        unsafe { read_register(self.base, 0x100 + offset) }
    }

    pub(crate) fn slot(&self) -> usize {
        let window = crate::arch::virtio_window();
        (self.base - window.start) / window.stride
    }

    pub(crate) fn base(&self) -> usize {
        self.base
    }

    pub(crate) const fn net_mac_feature() -> u64 {
        FEATURE_NET_MAC
    }
}

pub(crate) fn dispatch_interrupts() -> u64 {
    let window = crate::arch::virtio_window();
    let mut pending = 0u64;
    for slot in 0..window.slots.min(64) {
        let base = window.start + slot * window.stride;
        let magic = unsafe { read_register(base, REG_MAGIC) };
        let version = unsafe { read_register(base, REG_VERSION) };
        let device_id = unsafe { read_register(base, REG_DEVICE_ID) };
        if magic != MAGIC || version != VERSION_MODERN || device_id == 0 {
            continue;
        }
        let status = unsafe { read_register(base, REG_INTERRUPT_STATUS) };
        if status != 0 {
            unsafe { write_register(base, REG_INTERRUPT_ACK, status) };
            pending |= 1u64 << slot;
        }
    }
    pending
}

unsafe fn read_register(base: usize, offset: usize) -> u32 {
    core::ptr::read_volatile((base + offset) as *const u32)
}

unsafe fn write_register(base: usize, offset: usize, value: u32) {
    core::ptr::write_volatile((base + offset) as *mut u32, value);
}

unsafe fn write_address(base: usize, offset: usize, address: u64) {
    write_register(base, offset, address as u32);
    write_register(base, offset + 4, (address >> 32) as u32);
}
