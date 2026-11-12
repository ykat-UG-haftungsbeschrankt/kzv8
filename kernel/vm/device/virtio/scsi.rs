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

pub(crate) const MMIO_BASE: u64 = 0x1000_4000;
const DEVICE_ID: u32 = 8;
const VENDOR_ID: u32 = 0x4b5a;
const DISK_SIZE: usize = 4 * 1024 * 1024;
const SECTOR_SIZE: usize = 512;
const MAX_TRANSFER: usize = 64 * 1024;
const REQUEST_HEADER_SIZE: usize = 19;
const CDB_SIZE: usize = 32;
const REQUEST_SIZE: usize = REQUEST_HEADER_SIZE + CDB_SIZE + MAX_TRANSFER;
const RESPONSE_HEADER_SIZE: usize = 12;
const STATUS_GOOD: u8 = 0;
const STATUS_CHECK_CONDITION: u8 = 2;
const RESPONSE_OK: u8 = 0;
const RESPONSE_BAD_TARGET: u8 = 3;
const CDB_TEST_UNIT_READY: u8 = 0x00;
const CDB_INQUIRY: u8 = 0x12;
const CDB_READ_CAPACITY_10: u8 = 0x25;
const CDB_READ_10: u8 = 0x28;
const CDB_WRITE_10: u8 = 0x2a;

pub(crate) struct VirtioScsi {
    mmio: VirtioMmio,
    disk: Vec<u8>,
    request: Vec<u8>,
    response: Vec<u8>,
}

impl VirtioScsi {
    pub(crate) fn new() -> Option<Self> {
        let mut disk = Vec::new();
        disk.try_reserve_exact(DISK_SIZE).ok()?;
        disk.resize(DISK_SIZE, 0);
        let mut request = Vec::new();
        request.try_reserve_exact(REQUEST_SIZE).ok()?;
        request.resize(REQUEST_SIZE, 0);
        let mut response = Vec::new();
        response.try_reserve_exact(MAX_TRANSFER + RESPONSE_HEADER_SIZE).ok()?;
        response.resize(MAX_TRANSFER + RESPONSE_HEADER_SIZE, 0);
        let mut config = [0; 16];
        config[0] = 1;
        config[1] = 1;
        config[2] = (MAX_TRANSFER / SECTOR_SIZE) as u32;
        config[3] = 16;
        config[5] = 96;
        config[6] = CDB_SIZE as u32;
        config[7] = 0;
        config[8] = 0;
        Some(Self {
            mmio: VirtioMmio::new(
                MMIO_BASE,
                DEVICE_ID,
                VENDOR_ID,
                [0, 1],
                config,
                [0; 16],
                3,
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
        if queue_index > 2 {
            return QueueResult::Invalid;
        }
        if queue_index == 1 {
            return QueueResult::Pending;
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
            if readable_capacity > request.len()
                || chain.read_device_data(memory, request).is_none()
            {
                return QueueResult::Invalid;
            }
            let Some(writable_capacity) = chain.writable_capacity() else {
                return QueueResult::Invalid;
            };
            if queue_index == 0 {
                if readable_capacity < 24 || writable_capacity < 1 {
                    return QueueResult::Invalid;
                }
                let control_response = [11];
                if chain.write_device_data(memory, &control_response).is_none()
                    || !queue.complete(memory, &chain, 1)
                {
                    return QueueResult::Invalid;
                }
                completed = true;
                continue;
            }
            let used_length = Self::handle_request(
                &request[..readable_capacity],
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

    fn handle_request(
        request: &[u8],
        writable_capacity: usize,
        disk: &mut [u8],
        response: &mut [u8],
    ) -> usize {
        response.fill(0);
        if writable_capacity < RESPONSE_HEADER_SIZE {
            return 0;
        }
        if request.len() < REQUEST_HEADER_SIZE + CDB_SIZE {
            response[10] = STATUS_CHECK_CONDITION;
            response[11] = RESPONSE_BAD_TARGET;
            return RESPONSE_HEADER_SIZE;
        }
        if request[0] != 1 || request[1] != 0 || request[4..6] != [0, 0] {
            response[10] = STATUS_CHECK_CONDITION;
            response[11] = RESPONSE_BAD_TARGET;
            return RESPONSE_HEADER_SIZE;
        }
        let cdb = &request[REQUEST_HEADER_SIZE..REQUEST_HEADER_SIZE + CDB_SIZE];
        let mut data_length = 0usize;
        let mut status = STATUS_GOOD;
        let opcode = cdb[0];
        match opcode {
            CDB_TEST_UNIT_READY => {}
            CDB_INQUIRY => {
                data_length = usize::from(cdb[4]).min(36);
                if data_length >= 1 { response[0] = 0; }
                if data_length >= 2 { response[1] = 0; }
                if data_length >= 3 { response[2] = 6; }
                if data_length >= 4 { response[3] = 2; }
                if data_length >= 5 { response[4] = 31; }
                if data_length >= 16 { response[8..16].copy_from_slice(b"KZKERNEL"); }
                if data_length >= 32 { response[16..32].copy_from_slice(b"PER-VM RAM DISK  "); }
                if data_length >= 36 { response[32..36].copy_from_slice(b"0001"); }
            }
            CDB_READ_CAPACITY_10 => {
                data_length = 8;
                let last_lba = (disk.len() / SECTOR_SIZE - 1) as u32;
                response[..4].copy_from_slice(&last_lba.to_be_bytes());
                response[4..8].copy_from_slice(&(SECTOR_SIZE as u32).to_be_bytes());
            }
            CDB_READ_10 | CDB_WRITE_10 => {
                let lba = u32::from_be_bytes([cdb[2], cdb[3], cdb[4], cdb[5]]) as usize;
                let sectors = usize::from(u16::from_be_bytes([cdb[7], cdb[8]]));
                data_length = match sectors.checked_mul(SECTOR_SIZE) {
                    Some(length) if length <= MAX_TRANSFER => length,
                    _ => {
                        response[10] = STATUS_CHECK_CONDITION;
                        return RESPONSE_HEADER_SIZE;
                    }
                };
                let Some(offset) = lba.checked_mul(SECTOR_SIZE) else {
                    response[10] = STATUS_CHECK_CONDITION;
                    return RESPONSE_HEADER_SIZE;
                };
                let Some(end) = offset.checked_add(data_length) else {
                    response[10] = STATUS_CHECK_CONDITION;
                    return RESPONSE_HEADER_SIZE;
                };
                if data_length == 0 || end > disk.len() {
                    response[10] = STATUS_CHECK_CONDITION;
                    return RESPONSE_HEADER_SIZE;
                }
                if opcode == CDB_READ_10 {
                    response[..data_length]
                        .copy_from_slice(&disk[offset..end]);
                } else {
                    let request_data = &request[REQUEST_HEADER_SIZE + CDB_SIZE..];
                    if request_data.len() < data_length {
                        status = STATUS_CHECK_CONDITION;
                        data_length = 0;
                    } else {
                        disk[offset..end]
                            .copy_from_slice(&request_data[..data_length]);
                        data_length = 0;
                    }
                }
            }
            _ => status = STATUS_CHECK_CONDITION,
        }

        if data_length > writable_capacity.saturating_sub(RESPONSE_HEADER_SIZE) {
            status = STATUS_CHECK_CONDITION;
            data_length = 0;
        }
        let response_header = data_length;
        response[response_header..response_header + RESPONSE_HEADER_SIZE].fill(0);
        response[response_header + 10] = status;
        response[response_header + 11] = RESPONSE_OK;
        data_length + RESPONSE_HEADER_SIZE
    }
}