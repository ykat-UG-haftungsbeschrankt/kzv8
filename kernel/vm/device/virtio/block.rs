/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use crate::kernel::vm::device::virtio::mmio::{QueueResult, VirtioMmio};
use crate::kernel::vm::device::virtio::queue::GuestVirtQueue;
use crate::kernel::vm::device::GuestMemory;

pub(crate) const MMIO_BASE: u64 = 0x1000_2000;
const DEVICE_ID: u32 = 2;
const VENDOR_ID: u32 = 0x4b5a;
const FEATURE_FLUSH: u32 = 1 << 9;
const DISK_SIZE: usize = 4 * 1024 * 1024;
const SECTOR_SIZE: usize = 512;
const MAX_TRANSFER: usize = 64 * 1024;
const HEADER_SIZE: usize = 16;
const STATUS_OK: u8 = 0;
const STATUS_IOERR: u8 = 1;
const STATUS_UNSUPPORTED: u8 = 2;
const REQUEST_IN: u32 = 0;
const REQUEST_OUT: u32 = 1;
const REQUEST_FLUSH: u32 = 4;
const REQUEST_GET_ID: u32 = 8;

pub(crate) struct VirtioBlock {
    mmio: VirtioMmio,
    //@FIXME: INDEXTYPE(disk); fixed-size byte-addressed backing store; replace Vec with Box<[u8]> to express fixed capacity while retaining O(1) indexing.
    disk: Vec<u8>,
    //@FIXME: INDEXTYPE(request); fixed-capacity request scratch buffer; replace Vec with Box<[u8]> to express fixed size while retaining O(1) indexing.
    request: Vec<u8>,
    //@FIXME: INDEXTYPE(response); fixed-capacity response scratch buffer; replace Vec with Box<[u8]> to express fixed size while retaining O(1) indexing.
    response: Vec<u8>,
}

impl VirtioBlock {
    pub(crate) fn new() -> Option<Self> {
        let mut disk = Vec::new();
        disk.try_reserve_exact(DISK_SIZE).ok()?;
        disk.resize(DISK_SIZE, 0);
        let mut request = Vec::new();
        request.try_reserve_exact(HEADER_SIZE + MAX_TRANSFER).ok()?;
        request.resize(HEADER_SIZE + MAX_TRANSFER, 0);
        let mut response = Vec::new();
        response.try_reserve_exact(MAX_TRANSFER + 1).ok()?;
        response.resize(MAX_TRANSFER + 1, 0);
        let sectors = (DISK_SIZE / SECTOR_SIZE) as u64;
        let mut config = [0; 16];
        config[0] = sectors as u32;
        config[1] = (sectors >> 32) as u32;
        Some(Self {
            mmio: VirtioMmio::new(
                MMIO_BASE,
                DEVICE_ID,
                VENDOR_ID,
                [FEATURE_FLUSH, 1],
                config,
                [0; 16],
                1,
                256,
            ),
            disk,
            request,
            response,
        })
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
        let mmio = &mut self.mmio;
        let disk = &mut self.disk;
        let request = &mut self.request;
        let response = &mut self.response;
        mmio.access(memory, address, width, write, value, |queue_index, queue, memory| {
            Self::notify_queue(queue_index, queue, memory, disk, request, response)
        })
    }

    fn notify_queue(
        queue_index: usize,
        queue: &mut GuestVirtQueue,
        memory: &mut GuestMemory<'_>,
        disk: &mut [u8],
        request: &mut [u8],
        response: &mut [u8],
    ) -> QueueResult {
        if queue_index != 0 {
            return QueueResult::Invalid;
        }
        let mut completed = false;
        loop {
            let chain = match queue.available_chain(memory) {
                Ok(Some(chain)) => chain,
                Ok(None) => break,
                Err(()) => return QueueResult::Invalid,
            };
            let Some(readable_capacity) = chain.readable_capacity() else {
                return QueueResult::Invalid;
            };
            if readable_capacity > HEADER_SIZE + MAX_TRANSFER
                || chain.read_device_data(memory, request).is_none()
            {
                return QueueResult::Invalid;
            }
            let Some(request_length) = Self::request_length(request, readable_capacity) else {
                return QueueResult::Invalid;
            };
            let Some(writable_capacity) = chain.writable_capacity() else {
                return QueueResult::Invalid;
            };
            let used_length = Self::handle_request(
                &request[..request_length],
                writable_capacity,
                disk,
                response,
            );
            if used_length > writable_capacity {
                return QueueResult::Invalid;
            }
            let Some(written) = chain.write_device_data(memory, &response[..used_length]) else {
                return QueueResult::Invalid;
            };
            if !queue.complete(memory, &chain, written) {
                return QueueResult::Invalid;
            }
            completed = true;
        }
        if completed {
            QueueResult::Complete
        } else {
            QueueResult::Pending
        }
    }

    fn request_length(request: &[u8], readable_capacity: usize) -> Option<usize> {
        if readable_capacity < HEADER_SIZE || request.len() < HEADER_SIZE {
            return None;
        }
        let request_type = u32::from_le_bytes(request.get(..4)?.try_into().ok()?);
        match request_type {
            REQUEST_IN | REQUEST_FLUSH | REQUEST_GET_ID => Some(HEADER_SIZE),
            REQUEST_OUT => {
                let length = readable_capacity.checked_sub(HEADER_SIZE)?;
                (length <= MAX_TRANSFER).then_some(readable_capacity)
            }
            _ => Some(HEADER_SIZE),
        }
    }

    fn handle_request(
        request: &[u8],
        writable_capacity: usize,
        disk: &mut [u8],
        response: &mut [u8],
    ) -> usize {
        response.fill(0);
        if writable_capacity == 0 || request.len() < HEADER_SIZE {
            return 0;
        }
        response[0] = STATUS_UNSUPPORTED;
        let Ok(type_bytes) = <[u8; 4]>::try_from(&request[0..4]) else {
            response[0] = STATUS_IOERR;
            return 1;
        };
        let Ok(sector_bytes) = <[u8; 8]>::try_from(&request[8..16]) else {
            response[0] = STATUS_IOERR;
            return 1;
        };
        let request_type = u32::from_le_bytes(type_bytes);
        let sector = u64::from_le_bytes(sector_bytes);
        match request_type {
            REQUEST_IN => {
                let data_length = writable_capacity.saturating_sub(1);
                if !Self::valid_range(disk, sector, data_length) {
                    response[0] = STATUS_IOERR;
                    return 1;
                }
                let byte_offset = (sector as usize) * SECTOR_SIZE;
                response[..data_length].copy_from_slice(&disk[byte_offset..byte_offset + data_length]);
                response[data_length] = STATUS_OK;
                data_length + 1
            }
            REQUEST_OUT => {
                let data_length = request.len() - HEADER_SIZE;
                if writable_capacity < 1
                    || !Self::valid_range(disk, sector, data_length)
                    || data_length == 0
                {
                    response[0] = STATUS_IOERR;
                    return 1;
                }
                let byte_offset = (sector as usize) * SECTOR_SIZE;
                disk[byte_offset..byte_offset + data_length]
                    .copy_from_slice(&request[HEADER_SIZE..HEADER_SIZE + data_length]);
                response[0] = STATUS_OK;
                1
            }
            REQUEST_FLUSH => {
                response[0] = STATUS_OK;
                1
            }
            REQUEST_GET_ID => {
                let identifier = b"KZ-VIRTIO-RAMDISK";
                const ID_LENGTH: usize = 20;
                if writable_capacity < ID_LENGTH + 1 || response.len() < ID_LENGTH + 1 {
                    response[0] = STATUS_IOERR;
                    return 1;
                }
                response[..ID_LENGTH].fill(b' ');
                response[..identifier.len()].copy_from_slice(identifier);
                response[ID_LENGTH] = STATUS_OK;
                ID_LENGTH + 1
            }
            _ => 1,
        }
    }

    fn valid_range(disk: &[u8], sector: u64, length: usize) -> bool {
        length != 0
            && length % SECTOR_SIZE == 0
            && length <= MAX_TRANSFER
            && sector
                .checked_mul(SECTOR_SIZE as u64)
                .and_then(|offset| usize::try_from(offset).ok())
                .and_then(|offset| offset.checked_add(length))
                .is_some_and(|end| end <= disk.len())
    }
}