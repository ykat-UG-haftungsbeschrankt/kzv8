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

pub(crate) const MMIO_BASE: u64 = 0x1000_5000;
const DEVICE_ID: u32 = 26;
const VENDOR_ID: u32 = 0x4b5a;
const ROOT_ID: u64 = 1;
const FILE_ID: u64 = 2;
const REQUEST_QUEUE: usize = 2;
const REQUEST_CAPACITY: usize = 16 * 1024;
const RESPONSE_CAPACITY: usize = 16 * 1024;
const REQUEST_HEADER_SIZE: usize = 40;
const RESPONSE_HEADER_SIZE: usize = 16;
const INIT: u32 = 26;
const LOOKUP: u32 = 1;
const GETATTR: u32 = 3;
const OPEN: u32 = 14;
const READ: u32 = 15;
const WRITE: u32 = 16;
const RELEASE: u32 = 18;
const FUSE_FORGET: u32 = 2;
const FUSE_BATCH_FORGET: u32 = 42;
const OPENDIR: u32 = 27;
const READDIR: u32 = 28;
const RELEASEDIR: u32 = 29;
const DIRENT_HEADER_SIZE: usize = 24;
const DIRECTORY_MODE: u32 = 0o040555;
const FILE_MODE: u32 = 0o100444;
const DIRECTORY_TYPE: u32 = 4;
const FILE_TYPE: u32 = 8;
const FILE_BYTES: &[u8] = b"Hello from the per-VM VirtIO filesystem.\n";
const MAX_WRITE: u32 = 16 * 1024;

pub(crate) struct VirtioFs {
    mmio: VirtioMmio,
    //@FIXME: INDEXTYPE(request); fixed-capacity request scratch buffer; replace Vec with Box<[u8]> to express fixed size while retaining O(1) indexing.
    request: Vec<u8>,
    //@FIXME: INDEXTYPE(response); fixed-capacity response scratch buffer; replace Vec with Box<[u8]> to express fixed size while retaining O(1) indexing.
    response: Vec<u8>,
}

impl VirtioFs {
    pub(crate) fn new() -> Option<Self> {
        let mut config = [0; 16];
        let tag = b"kzguest";
        let mut tag_bytes = [0; 36];
        tag_bytes[..tag.len()].copy_from_slice(tag);
        for (index, chunk) in tag_bytes.chunks_exact(4).enumerate() {
            config[index] = u32::from_le_bytes(chunk.try_into().ok()?);
        }
        config[9] = 1;
        let mut request = Vec::new();
        request.try_reserve_exact(REQUEST_HEADER_SIZE + REQUEST_CAPACITY).ok()?;
        request.resize(REQUEST_HEADER_SIZE + REQUEST_CAPACITY, 0);
        let mut response = Vec::new();
        response.try_reserve_exact(RESPONSE_HEADER_SIZE + RESPONSE_CAPACITY).ok()?;
        response.resize(RESPONSE_HEADER_SIZE + RESPONSE_CAPACITY, 0);
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
        let request = &mut self.request;
        let response = &mut self.response;
        mmio.access(memory, address, width, write, value, |queue_index, queue, memory| {
            Self::notify_queue(queue_index, queue, memory, request, response)
        })
    }

    fn notify_queue(
        queue_index: usize,
        queue: &mut GuestVirtQueue,
        memory: &mut GuestMemory<'_>,
        request: &mut [u8],
        response: &mut [u8],
    ) -> QueueResult {
        if queue_index == 1 {
            return QueueResult::Pending;
        }
        if queue_index != 0 && queue_index != REQUEST_QUEUE {
            return QueueResult::Invalid;
        }
        let mut completed = false;
        loop {
            let chain = match queue.available_chain(memory) {
                Ok(Some(chain)) => chain,
                Ok(None) => break,
                Err(()) => return QueueResult::Invalid,
            };
            let Some(request_capacity) = chain.readable_capacity() else {
                return QueueResult::Invalid;
            };
            if request_capacity > request.len()
                || chain.read_device_data(memory, request).is_none()
            {
                return QueueResult::Invalid;
            }
            if queue_index == 0 {
                if request_capacity < REQUEST_HEADER_SIZE {
                    return QueueResult::Invalid;
                }
                let opcode = u32::from_le_bytes([request[4], request[5], request[6], request[7]]);
                if !matches!(opcode, FUSE_FORGET | FUSE_BATCH_FORGET) {
                    return QueueResult::Invalid;
                }
                if !queue.complete(memory, &chain, 0) {
                    return QueueResult::Invalid;
                }
                completed = true;
                continue;
            }
            let Some(response_length) = Self::handle_request(
                &request[..request_capacity],
                response,
            ) else {
                return QueueResult::Invalid;
            };
            if response_length > chain.writable_capacity().unwrap_or(0) {
                return QueueResult::Invalid;
            }
            let Some(written) = chain.write_device_data(memory, &response[..response_length]) else {
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

    fn handle_request(request: &[u8], response: &mut [u8]) -> Option<usize> {
        if request.len() < REQUEST_HEADER_SIZE || response.len() < RESPONSE_HEADER_SIZE {
            return None;
        }
        let declared_length = Self::read_u32(request, 0)? as usize;
        if declared_length != request.len() {
            return None;
        }
        let opcode = Self::read_u32(request, 4)?;
        let unique = Self::read_u64(request, 8)?;
        let node_id = Self::read_u64(request, 16)?;
        response.fill(0);
        let body_length = match opcode {
            INIT => Self::init_response(request, response),
            LOOKUP => Self::lookup_response(request, response, node_id),
            GETATTR => Self::getattr_response(response, node_id),
            OPEN => Self::open_response(request, response, node_id, false),
            OPENDIR => Self::open_response(request, response, node_id, true),
            READ => Self::read_response(request, response, node_id),
            READDIR => Self::readdir_response(request, response, node_id),
            RELEASE | RELEASEDIR => Some(0),
            WRITE => {
                Self::set_error(response, -30);
                Some(0)
            }
            _ => {
                Self::set_error(response, -38);
                Some(0)
            }
        }?;
        let response_length = RESPONSE_HEADER_SIZE.checked_add(body_length)?;
        Self::write_u32(response, 0, u32::try_from(response_length).ok()?);
        Self::write_u64(response, 8, unique);
        Some(response_length)
    }

    fn init_response(request: &[u8], response: &mut [u8]) -> Option<usize> {
        if request.len() < REQUEST_HEADER_SIZE + 8 {
            Self::set_error(response, -22);
            return Some(0);
        }
        let major = Self::read_u32(request, REQUEST_HEADER_SIZE)?;
        let minor = Self::read_u32(request, REQUEST_HEADER_SIZE + 4)?;
        if major != 7 {
            Self::set_error(response, -22);
            return Some(0);
        }
        let body = RESPONSE_HEADER_SIZE;
        Self::write_u32(response, body, 7);
        Self::write_u32(response, body + 4, minor.min(41));
        Self::write_u32(response, body + 20, MAX_WRITE);
        Some(64)
    }

    fn lookup_response(request: &[u8], response: &mut [u8], parent: u64) -> Option<usize> {
        let name = request.get(REQUEST_HEADER_SIZE..)?;
        let name_length = name.iter().position(|byte| *byte == 0).unwrap_or(name.len());
        if parent != ROOT_ID {
            Self::set_error(response, -20);
            return Some(0);
        }
        if &name[..name_length] != b"hello.txt" {
            Self::set_error(response, -2);
            return Some(0);
        }
        let body = RESPONSE_HEADER_SIZE;
        Self::write_u64(response, body, FILE_ID);
        Self::write_u64(response, body + 8, 1);
        Self::write_u64(response, body + 16, 1);
        Self::write_u64(response, body + 24, 1);
        Self::write_u32(response, body + 100, FILE_MODE);
        Self::write_u32(response, body + 104, 1);
        Self::write_u64(response, body + 48, FILE_BYTES.len() as u64);
        Some(128)
    }

    fn getattr_response(response: &mut [u8], node_id: u64) -> Option<usize> {
        let body = RESPONSE_HEADER_SIZE;
        let (mode, size) = match node_id {
            ROOT_ID => (DIRECTORY_MODE, 0),
            FILE_ID => (FILE_MODE, FILE_BYTES.len() as u64),
            _ => {
                Self::set_error(response, -2);
                return Some(0);
            }
        };
        Self::write_u64(response, body + 16, node_id);
        Self::write_u64(response, body + 24, size);
        Self::write_u64(response, body + 32, size.div_ceil(512));
        Self::write_u32(response, body + 76, mode);
        Self::write_u32(response, body + 80, 1);
        Self::write_u32(response, body + 96, 512);
        Some(104)
    }

    fn open_response(
        request: &[u8],
        response: &mut [u8],
        node_id: u64,
        directory: bool,
    ) -> Option<usize> {
        let valid = if directory { node_id == ROOT_ID } else { node_id == FILE_ID };
        if !valid {
            Self::set_error(response, if node_id == ROOT_ID { -21 } else { -2 });
            return Some(0);
        }
        if !directory
            && request.len() >= REQUEST_HEADER_SIZE + 4
            && Self::read_u32(request, REQUEST_HEADER_SIZE)? & 3 != 0
        {
            Self::set_error(response, -30);
            return Some(0);
        }
        Self::write_u64(response, RESPONSE_HEADER_SIZE, node_id);
        Some(16)
    }

    fn read_response(request: &[u8], response: &mut [u8], node_id: u64) -> Option<usize> {
        if node_id != FILE_ID || request.len() < REQUEST_HEADER_SIZE + 40 {
            Self::set_error(response, if node_id == FILE_ID { -22 } else { -2 });
            return Some(0);
        }
        let payload = REQUEST_HEADER_SIZE;
        let offset = usize::try_from(Self::read_u64(request, payload + 8)?).ok()?;
        let requested = Self::read_u32(request, payload + 16)? as usize;
        if offset >= FILE_BYTES.len() {
            return Some(0);
        }
        let count = requested.min(FILE_BYTES.len() - offset);
        response[RESPONSE_HEADER_SIZE..RESPONSE_HEADER_SIZE + count]
            .copy_from_slice(&FILE_BYTES[offset..offset + count]);
        Some(count)
    }

    fn readdir_response(request: &[u8], response: &mut [u8], node_id: u64) -> Option<usize> {
        if node_id != ROOT_ID || request.len() < REQUEST_HEADER_SIZE + 40 {
            Self::set_error(response, if node_id == ROOT_ID { -22 } else { -20 });
            return Some(0);
        }
        let payload = REQUEST_HEADER_SIZE;
        let offset = Self::read_u64(request, payload + 8)?;
        let capacity = Self::read_u32(request, payload + 16)? as usize;
        let body = RESPONSE_HEADER_SIZE;
        let mut written = 0;
        let entries: [(&[u8], u64, u32, u64); 3] = [
            (b".", ROOT_ID, DIRECTORY_TYPE, 1),
            (b"..", ROOT_ID, DIRECTORY_TYPE, 2),
            (b"hello.txt", FILE_ID, FILE_TYPE, 3),
        ];
        for (index, (name, inode, file_type, next_offset)) in entries.iter().enumerate() {
            if offset >= *next_offset {
                continue;
            }
            let record_length = (24 + name.len() + 7) & !7;
            if written + record_length > capacity || body + written + record_length > response.len() {
                break;
            }
            let record = body + written;
            Self::write_u64(response, record, *inode);
            Self::write_u64(response, record + 8, *next_offset);
            Self::write_u32(response, record + 16, name.len() as u32);
            Self::write_u32(response, record + 20, *file_type);
            response[record + 24..record + 24 + name.len()].copy_from_slice(name);
            written += record_length;
            if index + 1 == entries.len() {
                break;
            }
        }
        Some(written)
    }

    fn set_error(response: &mut [u8], error: i32) {
        Self::write_u32(response, 4, error as u32);
    }

    fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
    }

    fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
        Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
    }

    fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
}
