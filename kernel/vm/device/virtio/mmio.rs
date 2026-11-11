/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::vm::device::GuestMemory;
use crate::kernel::vm::device::virtio::queue::GuestVirtQueue;

const MMIO_SIZE: u64 = 0x1000;
const MAX_QUEUES: usize = 3;
const DEVICE_CONFIG_SIZE: usize = 16;
const STATUS_FEATURES_OK: u32 = 1 << 3;
const STATUS_DRIVER_OK: u32 = 1 << 2;
const STATUS_FAILED: u32 = 1 << 7;
const INTERRUPT_USED_BUFFER: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QueueResult {
    Complete,
    Pending,
    Invalid,
}

pub(crate) struct VirtioMmio {
    base: u64,
    device_id: u32,
    vendor_id: u32,
    device_features: [u32; 2],
    device_config: [u32; DEVICE_CONFIG_SIZE],
    config_writable: [u32; DEVICE_CONFIG_SIZE],
    queue_count: usize,
    queues: [GuestVirtQueue; MAX_QUEUES],
    device_features_select: u32,
    driver_features_select: u32,
    driver_features: [u32; 2],
    queue_select: u32,
    status: u32,
    interrupt_status: u32,
}

impl VirtioMmio {
    pub(crate) const fn new(
        base: u64,
        device_id: u32,
        vendor_id: u32,
        device_features: [u32; 2],
        device_config: [u32; DEVICE_CONFIG_SIZE],
        config_writable: [u32; DEVICE_CONFIG_SIZE],
        queue_count: usize,
        queue_max_size: u16,
    ) -> Self {
        assert!(
            queue_count > 0
                && queue_count <= MAX_QUEUES
                && queue_max_size.is_power_of_two()
        );
        Self {
            base,
            device_id,
            vendor_id,
            device_features,
            device_config,
            config_writable,
            queue_count,
            queues: [GuestVirtQueue::new(queue_max_size); MAX_QUEUES],
            device_features_select: 0,
            driver_features_select: 0,
            driver_features: [0; 2],
            queue_select: 0,
            status: 0,
            interrupt_status: 0,
        }
    }

    pub(crate) const fn contains(&self, address: u64) -> bool {
        address >= self.base && address < self.base + MMIO_SIZE
    }

    pub(crate) fn queue_mut(&mut self, index: usize) -> Option<&mut GuestVirtQueue> {
        (index < self.queue_count).then(|| &mut self.queues[index])
    }

    pub(crate) fn signal_used_buffer(&mut self) {
        self.interrupt_status |= INTERRUPT_USED_BUFFER;
    }

    pub(crate) fn write_device_config_word(&mut self, index: usize, value: u32) -> bool {
        let Some(config) = self.device_config.get_mut(index) else {
            return false;
        };
        let writable = self.config_writable[index];
        *config = (*config & !writable) | (value & writable);
        true
    }

    pub(crate) fn mark_failed(&mut self) {
        self.status |= STATUS_FAILED;
    }

    pub(crate) fn access(
        &mut self,
        memory: &mut GuestMemory<'_>,
        address: u64,
        width: u8,
        write: bool,
        value: u64,
        mut notify: impl FnMut(usize, &mut GuestVirtQueue, &mut GuestMemory<'_>) -> QueueResult,
    ) -> Option<u64> {
        if !self.contains(address) {
            return None;
        }
        let offset = address - self.base;
        if offset >= 0x100 {
            if !matches!(width, 1 | 2 | 4 | 8) {
                self.status |= STATUS_FAILED;
                return Some(0);
            }
            if write {
                self.write_device_config(offset, width, value);
                return Some(0);
            }
            return Some(self.read_device_config(offset, width).unwrap_or(0));
        }
        if width != 4 {
            self.status |= STATUS_FAILED;
            return Some(0);
        }
        if offset & 3 != 0 {
            self.status |= STATUS_FAILED;
            return Some(0);
        }
        if write {
            if offset == 0x050 {
                if self.status & STATUS_DRIVER_OK == 0 {
                    self.status |= STATUS_FAILED;
                    return Some(0);
                }
                self.notify_queue(memory, value as usize, &mut notify);
            } else {
                self.write_register(memory, offset, value as u32);
            }
            Some(0)
        } else {
            Some(u64::from(self.read_register(offset)))
        }
    }

    fn read_register(&self, offset: u64) -> u32 {
        match offset {
            0x000 => 0x7472_6976,
            0x004 => 2,
            0x008 => self.device_id,
            0x00c => self.vendor_id,
            0x010 => self
                .device_features
                .get(self.device_features_select as usize)
                .copied()
                .unwrap_or(0),
            0x020 => self
                .driver_features
                .get(self.driver_features_select as usize)
                .copied()
                .unwrap_or(0),
            0x030 => self.queue_select,
            0x034 => self
                .selected_queue()
                .map_or(0, |queue| u32::from(queue.max_size())),
            0x038 => self
                .selected_queue()
                .map_or(0, |queue| u32::from(queue.size())),
            0x044 => self
                .selected_queue()
                .map_or(0, |queue| if queue.is_ready() { 1 } else { 0 }),
            0x060 => self.interrupt_status,
            0x070 => self.status,
            0x080 => self.selected_queue().map_or(0, |queue| queue.address_part(0).unwrap_or(0)),
            0x084 => self.selected_queue().map_or(0, |queue| queue.address_part(1).unwrap_or(0)),
            0x090 => self.selected_queue().map_or(0, |queue| queue.address_part(2).unwrap_or(0)),
            0x094 => self.selected_queue().map_or(0, |queue| queue.address_part(3).unwrap_or(0)),
            0x0a0 => self.selected_queue().map_or(0, |queue| queue.address_part(4).unwrap_or(0)),
            0x0a4 => self.selected_queue().map_or(0, |queue| queue.address_part(5).unwrap_or(0)),
            _ => 0,
        }
    }

    fn write_register(&mut self, memory: &mut GuestMemory<'_>, offset: u64, value: u32) {
        match offset {
            0x014 => self.device_features_select = value,
            0x020 => self.write_driver_features(value),
            0x024 => self.driver_features_select = value,
            0x030 => self.queue_select = value,
            0x038 => {
                let valid = self
                    .selected_queue_mut()
                    .is_some_and(|queue| queue.set_size(value));
                if !valid {
                    self.status |= STATUS_FAILED;
                }
            }
            0x044 => {
                let valid = self
                    .selected_queue_mut()
                    .is_some_and(|queue| queue.set_ready(value, memory));
                if !valid {
                    self.status |= STATUS_FAILED;
                }
            }
            0x064 => self.interrupt_status &= !value,
            0x070 => self.write_status(value),
            0x080 => self.set_queue_address(0, value),
            0x084 => self.set_queue_address(1, value),
            0x090 => self.set_queue_address(2, value),
            0x094 => self.set_queue_address(3, value),
            0x0a0 => self.set_queue_address(4, value),
            0x0a4 => self.set_queue_address(5, value),
            _ => {}
        }
    }

    fn read_device_config(&self, offset: u64, width: u8) -> Option<u64> {
        let byte_offset = usize::try_from(offset.checked_sub(0x100)?).ok()?;
        let byte_length = usize::from(width);
        if byte_offset.checked_add(byte_length)? > DEVICE_CONFIG_SIZE * 4 {
            return None;
        }
        let mut value = 0u64;
        for index in 0..byte_length {
            let position = byte_offset + index;
            let word = *self.device_config.get(position / 4)?;
            let byte = (word >> ((position % 4) * 8)) & 0xff;
            value |= u64::from(byte) << (index * 8);
        }
        Some(value)
    }

    fn write_device_config(&mut self, offset: u64, width: u8, value: u64) {
        let Some(byte_offset) = offset.checked_sub(0x100).and_then(|offset| usize::try_from(offset).ok()) else {
            return;
        };
        let byte_length = usize::from(width);
        if byte_offset
            .checked_add(byte_length)
            .is_none_or(|end| end > DEVICE_CONFIG_SIZE * 4)
        {
            return;
        }
        for index in 0..byte_length {
            let position = byte_offset + index;
            let word = position / 4;
            let shift = (position % 4) * 8;
            let writable = (self.config_writable[word] >> shift) & 0xff;
            let mask = writable << shift;
            let byte = ((value >> (index * 8)) as u32) << shift;
            self.device_config[word] =
                (self.device_config[word] & !mask) | (byte & mask);
        }
    }

    fn write_driver_features(&mut self, value: u32) {
        let Some(features) = self.driver_features.get_mut(self.driver_features_select as usize)
        else {
            self.status |= STATUS_FAILED;
            return;
        };
        if value & !self.device_features[self.driver_features_select as usize] != 0 {
            self.status |= STATUS_FAILED;
        } else {
            *features = value;
        }
    }

    fn write_status(&mut self, value: u32) {
        if value == 0 {
            self.device_features_select = 0;
            self.driver_features_select = 0;
            self.driver_features = [0; 2];
            self.queue_select = 0;
            self.status = 0;
            self.interrupt_status = 0;
            for queue in &mut self.queues {
                queue.reset();
            }
            return;
        }
        if value & self.status != self.status {
            self.status |= STATUS_FAILED;
            return;
        }
        if value & STATUS_FEATURES_OK != 0
            && (self.driver_features[1] & 1 == 0 || self.driver_features[1] & !self.device_features[1] != 0)
        {
            self.status = (value & !STATUS_FEATURES_OK) | STATUS_FAILED;
        } else {
            self.status = value;
        }
    }

    fn set_queue_address(&mut self, field: u8, value: u32) {
        let valid = self
            .selected_queue_mut()
            .is_some_and(|queue| queue.set_address_part(field, value));
        if !valid {
            self.status |= STATUS_FAILED;
        }
    }

    fn notify_queue(
        &mut self,
        memory: &mut GuestMemory<'_>,
        queue_index: usize,
        notify: &mut impl FnMut(usize, &mut GuestVirtQueue, &mut GuestMemory<'_>) -> QueueResult,
    ) {
        if queue_index >= self.queue_count {
            self.status |= STATUS_FAILED;
            return;
        }
        match notify(queue_index, &mut self.queues[queue_index], memory) {
            QueueResult::Complete => self.interrupt_status |= INTERRUPT_USED_BUFFER,
            QueueResult::Pending => {}
            QueueResult::Invalid => self.status |= STATUS_FAILED,
        }
    }

    fn selected_queue(&self) -> Option<&GuestVirtQueue> {
        self.queues.get(self.queue_select as usize).filter(|_| {
            (self.queue_select as usize) < self.queue_count
        })
    }

    fn selected_queue_mut(&mut self) -> Option<&mut GuestVirtQueue> {
        let index = self.queue_select as usize;
        (index < self.queue_count).then(|| &mut self.queues[index])
    }
}