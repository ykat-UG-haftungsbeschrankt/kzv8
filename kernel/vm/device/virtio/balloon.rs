/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::vm::device::virtio::mmio::{QueueResult, VirtioMmio};
use crate::kernel::vm::device::virtio::queue::GuestVirtQueue;
use crate::kernel::vm::device::GuestMemory;

pub(crate) const MMIO_BASE: u64 = 0x1000_8000;
const DEVICE_ID: u32 = 5;
const VENDOR_ID: u32 = 0x4b5a;
const PAGE_SIZE: u64 = 4096;
const GUEST_MEMORY_SIZE: u64 = 2 * 1024 * 1024;
const PAGE_COUNT: u32 = (GUEST_MEMORY_SIZE / PAGE_SIZE) as u32;
const MAX_PAGES_PER_REQUEST: usize = 16;
const FEATURE_DEFLATE_ON_OOM: u32 = 1 << 2;

pub(crate) struct VirtioBalloon {
    mmio: VirtioMmio,
    guest_pfns: [bool; PAGE_COUNT as usize],
    actual_pages: u32,
}

impl VirtioBalloon {
    pub(crate) const fn new() -> Self {
        let mut config = [0; 16];
        config[0] = 0;
        config[1] = 0;
        let mut writable = [0; 16];
        writable[1] = u32::MAX;
        Self {
            mmio: VirtioMmio::new(
                MMIO_BASE,
                DEVICE_ID,
                VENDOR_ID,
                [FEATURE_DEFLATE_ON_OOM, 1],
                config,
                writable,
                2,
                256,
            ),
            guest_pfns: [false; PAGE_COUNT as usize],
            actual_pages: 0,
        }
    }

    pub(crate) const fn contains(address: u64) -> bool {
        address >= MMIO_BASE && address < MMIO_BASE + 0x1000
    }

    pub(crate) fn access(
        &mut self,
        memory: &mut GuestMemory<'_>,
        address: u64,
        width: u8,
        write: bool,
        value: u64,
    ) -> Option<u64> {
        if !Self::contains(address) {
            return None;
        }
        if address >= MMIO_BASE + 0x100 {
            return self.access_config(address - MMIO_BASE - 0x100, width, write, value);
        }
        let mmio = &mut self.mmio;
        let guest_pfns = &mut self.guest_pfns;
        let actual_pages = &mut self.actual_pages;
        let result = mmio.access(memory, address, width, write, value, |queue_index, queue, memory| {
            Self::notify_queue(queue_index, queue, memory, guest_pfns, actual_pages)
        });
        let _updated = mmio.write_device_config_word(1, *actual_pages);
        result
    }

    fn access_config(&mut self, offset: u64, width: u8, write: bool, value: u64) -> Option<u64> {
        if !matches!(width, 1 | 2 | 4 | 8) {
            return Some(0);
        }
        let start = usize::try_from(offset).ok()?;
        let length = usize::from(width);
        let end = start.checked_add(length)?;
        if end > 64 {
            return Some(0);
        }
        let mut config = [0; 8];
        config[4..8].copy_from_slice(&self.actual_pages.to_le_bytes());
        if write {
            if start < 4 || end > 8 {
                self.mmio.mark_failed();
                return Some(0);
            }
            for index in 0..length {
                config[start + index] = (value >> (index * 8)) as u8;
            }
            let actual = u32::from_le_bytes(config[4..8].try_into().ok()?);
            if actual != self.actual_pages {
                self.mmio.mark_failed();
            } else {
                let _updated = self.mmio.write_device_config_word(1, actual);
            }
            return Some(0);
        }
        let mut result = 0u64;
        for index in 0..length {
            result |= u64::from(config[start + index]) << (index * 8);
        }
        Some(result)
    }

    fn notify_queue(
        queue_index: usize,
        queue: &mut GuestVirtQueue,
        memory: &mut GuestMemory<'_>,
        guest_pfns: &mut [bool; PAGE_COUNT as usize],
        actual_pages: &mut u32,
    ) -> QueueResult {
        if queue_index > 1 {
            return QueueResult::Invalid;
        }
        let mut completed = false;
        loop {
            let chain = match queue.available_chain(memory) {
                Ok(Some(chain)) => chain,
                Ok(None) => break,
                Err(()) => return QueueResult::Invalid,
            };
            let Some(capacity) = chain.readable_capacity() else {
                return QueueResult::Invalid;
            };
            if capacity == 0
                || capacity % core::mem::size_of::<u32>() != 0
                || capacity / 4 > MAX_PAGES_PER_REQUEST
            {
                return QueueResult::Invalid;
            }
            let mut bytes = [0; MAX_PAGES_PER_REQUEST * 4];
            if chain.read_device_data(memory, &mut bytes[..capacity]).is_none() {
                return QueueResult::Invalid;
            }
            let mut request_pfns = [0u32; MAX_PAGES_PER_REQUEST];
            let pfn_count = capacity / 4;
            for (index, chunk) in bytes[..capacity].chunks_exact(4).enumerate() {
                let pfn = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                if pfn >= PAGE_COUNT {
                    return QueueResult::Invalid;
                }
                if request_pfns[..index].contains(&pfn)
                    || (queue_index == 0 && guest_pfns[pfn as usize])
                    || (queue_index == 1 && !guest_pfns[pfn as usize])
                {
                    return QueueResult::Invalid;
                }
                request_pfns[index] = pfn;
            }
            if !queue.complete(memory, &chain, 0) {
                return QueueResult::Invalid;
            }
            for pfn in request_pfns.iter().take(pfn_count).copied() {
                if queue_index == 0 {
                    guest_pfns[pfn as usize] = true;
                    *actual_pages = (*actual_pages).saturating_add(1);
                } else {
                    guest_pfns[pfn as usize] = false;
                    *actual_pages = (*actual_pages).saturating_sub(1);
                }
            }
            completed = true;
        }
        if completed {
            QueueResult::Complete
        } else {
            QueueResult::Pending
        }
    }
}
