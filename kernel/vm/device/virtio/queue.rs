/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::sync::atomic::{fence, Ordering};

use crate::kernel::vm::device::GuestMemory;

const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2;

#[derive(Clone, Copy)]
struct Descriptor {
    address: u64,
    length: usize,
    flags: u16,
    next: u16,
}

pub(crate) struct GuestChain {
    head: u16,
    //@FIXME: INDEXTYPE(descriptors); variable-length descriptor chain traversed in order; Vec gives O(1) indexing and amortized O(1) append, keep Vec.
    descriptors: Vec<Descriptor>,
    length: usize,
}

impl GuestChain {
    pub(crate) fn readable_capacity(&self) -> Option<usize> {
        let mut capacity = 0usize;
        let mut device_writes = false;
        for descriptor in self.descriptors.iter().take(self.length) {
            if descriptor.flags & DESC_WRITE != 0 {
                device_writes = true;
            } else {
                if device_writes {
                    return None;
                }
                capacity = capacity.checked_add(descriptor.length)?;
            }
        }
        Some(capacity)
    }

    pub(crate) fn writable_capacity(&self) -> Option<usize> {
        let mut capacity = 0usize;
        let mut device_writes = false;
        for descriptor in self.descriptors.iter().take(self.length) {
            if descriptor.flags & DESC_WRITE != 0 {
                device_writes = true;
                capacity = capacity.checked_add(descriptor.length)?;
            } else if device_writes {
                return None;
            }
        }
        Some(capacity)
    }

    pub(crate) fn read_device_data(
        &self,
        memory: &GuestMemory<'_>,
        output: &mut [u8],
    ) -> Option<usize> {
        let capacity = self.readable_capacity()?;
        if capacity > output.len() {
            return None;
        }
        let mut copied = 0usize;
        for descriptor in self.descriptors.iter().take(self.length) {
            if descriptor.flags & DESC_WRITE != 0 {
                break;
            }
            let end = copied.checked_add(descriptor.length)?;
            if !memory.read(descriptor.address, &mut output[copied..end]) {
                return None;
            }
            copied = end;
        }
        Some(copied)
    }

    pub(crate) fn write_device_data(
        &self,
        memory: &mut GuestMemory<'_>,
        input: &[u8],
    ) -> Option<usize> {
        let capacity = self.writable_capacity()?;
        if input.len() > capacity {
            return None;
        }
        let mut copied = 0usize;
        for descriptor in self.descriptors.iter().take(self.length) {
            if descriptor.flags & DESC_WRITE == 0 {
                continue;
            }
            let length = descriptor.length.min(input.len() - copied);
            if length != 0 && !memory.write(descriptor.address, &input[copied..copied + length]) {
                return None;
            }
            copied += length;
            if copied == input.len() {
                break;
            }
        }
        (copied == input.len()).then_some(copied)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct GuestVirtQueue {
    max_size: u16,
    size: u16,
    ready: bool,
    descriptors: u64,
    available: u64,
    used: u64,
    last_available_index: u16,
}

#[cfg(test)]
#[path = "../../../../tests/kernel/vm/device/virtio/queue.rs"]
mod tests;

impl GuestVirtQueue {
    pub(crate) const fn new(max_size: u16) -> Self {
        Self {
            max_size,
            size: 0,
            ready: false,
            descriptors: 0,
            available: 0,
            used: 0,
            last_available_index: 0,
        }
    }

    pub(crate) const fn max_size(&self) -> u16 {
        self.max_size
    }

    pub(crate) const fn size(&self) -> u16 {
        self.size
    }

    pub(crate) const fn is_ready(&self) -> bool {
        self.ready
    }

    pub(crate) fn reset(&mut self) {
        self.size = 0;
        self.ready = false;
        self.descriptors = 0;
        self.available = 0;
        self.used = 0;
        self.last_available_index = 0;
    }

    pub(crate) fn descriptor_address(&self) -> u64 {
        self.descriptors
    }

    pub(crate) fn available_address(&self) -> u64 {
        self.available
    }

    pub(crate) fn used_address(&self) -> u64 {
        self.used
    }

    pub(crate) fn set_size(&mut self, value: u32) -> bool {
        if self.ready || value == 0 || value > u32::from(self.max_size) || !value.is_power_of_two()
        {
            return false;
        }
        self.size = value as u16;
        true
    }

    pub(crate) fn set_address_part(&mut self, field: u8, value: u32) -> bool {
        if self.ready {
            return false;
        }
        let target = match field {
            0 | 1 => &mut self.descriptors,
            2 | 3 => &mut self.available,
            4 | 5 => &mut self.used,
            _ => return false,
        };
        let shift = if field & 1 == 0 { 0 } else { 32 };
        let mask = !(u64::from(u32::MAX) << shift);
        *target = (*target & mask) | (u64::from(value) << shift);
        true
    }

    pub(crate) fn address_part(&self, field: u8) -> Option<u32> {
        let address = match field {
            0 | 1 => self.descriptors,
            2 | 3 => self.available,
            4 | 5 => self.used,
            _ => return None,
        };
        Some(if field & 1 == 0 {
            address as u32
        } else {
            (address >> 32) as u32
        })
    }

    pub(crate) fn set_ready(&mut self, value: u32, memory: &GuestMemory<'_>) -> bool {
        match value {
            0 => {
                self.ready = false;
                self.last_available_index = 0;
                true
            }
            1 if !self.ready && self.is_valid(memory) => {
                self.ready = true;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn is_valid(&self, memory: &GuestMemory<'_>) -> bool {
        self.size != 0
            && self.size <= self.max_size
            && self.size.is_power_of_two()
            && self.descriptors & 0xf == 0
            && self.available & 1 == 0
            && self.used & 3 == 0
            && memory.contains(self.descriptors, usize::from(self.size) * 16)
            && memory.contains(self.available, 6 + usize::from(self.size) * 2)
            && memory.contains(self.used, 6 + usize::from(self.size) * 8)
    }

    pub(crate) fn available_chain(
        &self,
        memory: &GuestMemory<'_>,
    ) -> Result<Option<GuestChain>, ()> {
        if !self.ready {
            return Err(());
        }
        fence(Ordering::SeqCst);
        let Some(available_index) = memory.read_u16(self.available + 2) else {
            return Err(());
        };
        let pending = available_index.wrapping_sub(self.last_available_index);
        if pending > self.size {
            return Err(());
        }
        if pending == 0 {
            return Ok(None);
        }
        let ring_offset = 4 + u64::from(self.last_available_index % self.size) * 2;
        let Some(head) = memory.read_u16(self.available + ring_offset) else {
            return Err(());
        };
        if head >= self.size {
            return Err(());
        }
        let mut descriptors = Vec::new();
        descriptors.try_reserve_exact(usize::from(self.size)).map_err(|_| ())?;
        let queue_size = usize::from(self.size);
        let mut visited = Vec::new();
        visited.try_reserve_exact(queue_size).map_err(|_| ())?;
        visited.resize(queue_size, false);
        let mut chain = GuestChain {
            head,
            descriptors,
            length: 0,
        };
        let mut descriptor_index = head;
        for _ in 0..self.size {
            if descriptor_index >= self.size {
                return Err(());
            }
            let visited_slot = &mut visited[usize::from(descriptor_index)];
            if *visited_slot {
                return Err(());
            }
            *visited_slot = true;

            let address = self.descriptors + u64::from(descriptor_index) * 16;
            let Some(buffer_address) = memory.read_u64(address) else {
                return Err(());
            };
            let Some(length) = memory.read_u32(address + 8) else {
                return Err(());
            };
            let Some(flags) = memory.read_u16(address + 12) else {
                return Err(());
            };
            let Some(next) = memory.read_u16(address + 14) else {
                return Err(());
            };
            if flags & !(DESC_NEXT | DESC_WRITE) != 0
                || !memory.contains(buffer_address, length as usize)
            {
                return Err(());
            }
            chain.descriptors[chain.length] = Descriptor {
                address: buffer_address,
                length: length as usize,
                flags,
                next,
            };
            chain.length += 1;
            if flags & DESC_NEXT == 0 {
                return Ok(Some(chain));
            }
            descriptor_index = next;
        }
        Err(())
    }

    pub(crate) fn complete(
        &mut self,
        memory: &mut GuestMemory<'_>,
        chain: &GuestChain,
        length: usize,
    ) -> bool {
        let Some(used_index) = memory.read_u16(self.used + 2) else {
            return false;
        };
        let used_offset = 4 + u64::from(used_index % self.size) * 8;
        if !memory.write_u32(self.used + used_offset, u32::from(chain.head))
            || !memory.write_u32(self.used + used_offset + 4, length as u32)
        {
            return false;
        }
        fence(Ordering::SeqCst);
        if !memory.write_u16(self.used + 2, used_index.wrapping_add(1)) {
            return false;
        }
        self.last_available_index = self.last_available_index.wrapping_add(1);
        true
    }
}