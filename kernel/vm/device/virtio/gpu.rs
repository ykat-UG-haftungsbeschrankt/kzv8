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

pub(crate) const MMIO_BASE: u64 = 0x1000_7000;
const DEVICE_ID: u32 = 16;
const VENDOR_ID: u32 = 0x4b5a;
const MAX_WIDTH: usize = 640;
const MAX_HEIGHT: usize = 480;
const BYTES_PER_PIXEL: usize = 4;
const FRAMEBUFFER_SIZE: usize = MAX_WIDTH * MAX_HEIGHT * BYTES_PER_PIXEL;
const RESOURCE_ID: u32 = 1;
const REQUEST_CAPACITY: usize = 512;
const DISPLAY_COUNT: usize = 16;
const DISPLAY_INFO_SIZE: usize = HEADER_SIZE + DISPLAY_COUNT * 24;
const RESPONSE_CAPACITY: usize = DISPLAY_INFO_SIZE;
const HEADER_SIZE: usize = 24;

const CMD_GET_DISPLAY_INFO: u32 = 0x0100;
const CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
const CMD_RESOURCE_UNREF: u32 = 0x0102;
const CMD_SET_SCANOUT: u32 = 0x0103;
const CMD_RESOURCE_FLUSH: u32 = 0x0104;
const CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
const CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const CMD_RESOURCE_DETACH_BACKING: u32 = 0x0107;
const RESP_OK_NODATA: u32 = 0x1100;
const RESP_OK_DISPLAY_INFO: u32 = 0x1101;
const RESP_ERR_UNSPEC: u32 = 0x1200;
const RESP_ERR_INVALID_PARAMETER: u32 = 0x1201;
const FORMAT_B8G8R8A8_UNORM: u32 = 1;

pub(crate) struct VirtioGpu {
    mmio: VirtioMmio,
    //@FIXME: INDEXTYPE(request); fixed-capacity request scratch buffer; replace Vec with Box<[u8]> to express fixed size while retaining O(1) indexing.
    request: Vec<u8>,
    //@FIXME: INDEXTYPE(response); fixed-capacity response scratch buffer; replace Vec with Box<[u8]> to express fixed size while retaining O(1) indexing.
    response: Vec<u8>,
    //@FIXME: INDEXTYPE(framebuffer); fixed pixel backing buffer; replace Vec with Box<[u8]> to express fixed size while retaining O(1) indexing.
    framebuffer: Vec<u8>,
    width: usize,
    height: usize,
    resource_created: bool,
    backing_address: u64,
    backing_length: usize,
    scanout_enabled: bool,
}

impl VirtioGpu {
    pub(crate) fn new() -> Option<Self> {
        let mut framebuffer = Vec::new();
        framebuffer.try_reserve_exact(FRAMEBUFFER_SIZE).ok()?;
        framebuffer.resize(FRAMEBUFFER_SIZE, 0);
        let mut request = Vec::new();
        request.try_reserve_exact(REQUEST_CAPACITY).ok()?;
        request.resize(REQUEST_CAPACITY, 0);
        let mut response = Vec::new();
        response.try_reserve_exact(RESPONSE_CAPACITY).ok()?;
        response.resize(RESPONSE_CAPACITY, 0);
        let mut config = [0; 16];
        config[2] = 1;
        Some(Self {
            mmio: VirtioMmio::new(
                MMIO_BASE,
                DEVICE_ID,
                VENDOR_ID,
                [0, 1],
                config,
                [0; 16],
                2,
                256,
            ),
            request,
            response,
            framebuffer,
            width: MAX_WIDTH,
            height: MAX_HEIGHT,
            resource_created: false,
            backing_address: 0,
            backing_length: 0,
            scanout_enabled: false,
        })
    }

    pub(crate) const fn contains(address: u64) -> bool {
        address >= MMIO_BASE && address < MMIO_BASE + 0x1000
    }

    pub(crate) fn access(
        &mut self,
        memory: &mut GuestMemory<'_>,
        address: u64,
        access_width: u8,
        write: bool,
        value: u64,
    ) -> Option<u64> {
        if !Self::contains(address) {
            return None;
        }
        let mmio = &mut self.mmio;
        let request = &mut self.request;
        let response = &mut self.response;
        let framebuffer = &mut self.framebuffer;
        let width = &mut self.width;
        let height = &mut self.height;
        let resource_created = &mut self.resource_created;
        let backing_address = &mut self.backing_address;
        let backing_length = &mut self.backing_length;
        let scanout_enabled = &mut self.scanout_enabled;
        mmio.access(memory, address, access_width, write, value, |queue_index, queue, memory| {
            Self::notify_queue(
                queue_index,
                queue,
                memory,
                request,
                response,
                framebuffer,
                width,
                height,
                resource_created,
                backing_address,
                backing_length,
                scanout_enabled,
            )
        })
    }

    fn notify_queue(
        queue_index: usize,
        queue: &mut GuestVirtQueue,
        memory: &mut GuestMemory<'_>,
        request: &mut [u8],
        response: &mut [u8],
        framebuffer: &mut [u8],
        width: &mut usize,
        height: &mut usize,
        resource_created: &mut bool,
        backing_address: &mut u64,
        backing_length: &mut usize,
        scanout_enabled: &mut bool,
    ) -> QueueResult {
        if queue_index == 1 {
            return QueueResult::Pending;
        }
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
            let Some(request_length) = chain.readable_capacity() else {
                return QueueResult::Invalid;
            };
            if request_length > request.len()
                || chain.read_device_data(memory, request).is_none()
            {
                return QueueResult::Invalid;
            }
            let response_length = Self::handle_command(
                &request[..request_length],
                response,
                memory,
                framebuffer,
                width,
                height,
                resource_created,
                backing_address,
                backing_length,
                scanout_enabled,
            );
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

    fn handle_command(
        request: &[u8],
        response: &mut [u8],
        memory: &GuestMemory<'_>,
        framebuffer: &mut [u8],
        width: &mut usize,
        height: &mut usize,
        resource_created: &mut bool,
        backing_address: &mut u64,
        backing_length: &mut usize,
        scanout_enabled: &mut bool,
    ) -> usize {
        response.fill(0);
        if request.len() < HEADER_SIZE || response.len() < HEADER_SIZE {
            return 0;
        }
        let Some(command) = Self::read_u32(request, 0) else {
            return Self::error(response, RESP_ERR_INVALID_PARAMETER);
        };
        let response_length = match command {
            CMD_GET_DISPLAY_INFO => {
                if request.len() < HEADER_SIZE {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                }
                Self::write_u32(response, 0, RESP_OK_DISPLAY_INFO);
                let display = HEADER_SIZE;
                Self::write_rect(response, display, 0, 0, *width as u32, *height as u32);
                Self::write_u32(response, display + 16, 1);
                DISPLAY_INFO_SIZE
            }
            CMD_RESOURCE_CREATE_2D => {
                if request.len() < 40
                    || Self::read_u32(request, 24) != Some(RESOURCE_ID)
                    || Self::read_u32(request, 28) != Some(FORMAT_B8G8R8A8_UNORM)
                {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                }
                let Some(new_width) = Self::read_u32(request, 32).map(|value| value as usize) else {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                };
                let Some(new_height) = Self::read_u32(request, 36).map(|value| value as usize) else {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                };
                if new_width == 0
                    || new_height == 0
                    || new_width > MAX_WIDTH
                    || new_height > MAX_HEIGHT
                    || new_width.checked_mul(new_height).and_then(|pixels| pixels.checked_mul(4))
                        .is_none_or(|length| length > framebuffer.len())
                {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                }
                *width = new_width;
                *height = new_height;
                *resource_created = true;
                Self::write_u32(response, 0, RESP_OK_NODATA);
                HEADER_SIZE
            }
            CMD_RESOURCE_UNREF => {
                if request.len() < 32 || Self::read_u32(request, 24) != Some(RESOURCE_ID) {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                }
                *resource_created = false;
                *backing_address = 0;
                *backing_length = 0;
                *scanout_enabled = false;
                Self::write_u32(response, 0, RESP_OK_NODATA);
                HEADER_SIZE
            }
            CMD_RESOURCE_ATTACH_BACKING => {
                if request.len() < 48
                    || !*resource_created
                    || Self::read_u32(request, 24) != Some(RESOURCE_ID)
                    || Self::read_u32(request, 28) != Some(1)
                {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                }
                let Some(guest_address) = Self::read_u64(request, 32) else {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                };
                let Some(length) = Self::read_u32(request, 40).map(|value| value as usize) else {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                };
                let Some(required) = width.checked_mul(*height).and_then(|pixels| pixels.checked_mul(4)) else {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                };
                if length < required || !memory.contains(guest_address, required) {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                }
                *backing_address = guest_address;
                *backing_length = length;
                Self::write_u32(response, 0, RESP_OK_NODATA);
                HEADER_SIZE
            }
            CMD_SET_SCANOUT => {
                if request.len() < 48
                    || !*resource_created
                    || Self::read_u32(request, 40) != Some(0)
                    || Self::read_u32(request, 44) != Some(RESOURCE_ID)
                {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                }
                *scanout_enabled = true;
                Self::write_u32(response, 0, RESP_OK_NODATA);
                HEADER_SIZE
            }
            CMD_TRANSFER_TO_HOST_2D => {
                if request.len() < 56
                    || !*resource_created
                    || *backing_address == 0
                    || Self::read_u32(request, 48) != Some(RESOURCE_ID)
                {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                }
                let Some(x) = Self::read_u32(request, 24).map(|value| value as usize) else {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                };
                let Some(y) = Self::read_u32(request, 28).map(|value| value as usize) else {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                };
                let Some(rect_width) = Self::read_u32(request, 32).map(|value| value as usize) else {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                };
                let Some(rect_height) = Self::read_u32(request, 36).map(|value| value as usize) else {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                };
                let Some(offset) = Self::read_u64(request, 40).and_then(|value| usize::try_from(value).ok()) else {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                };
                if rect_width == 0
                    || rect_height == 0
                    || x.checked_add(rect_width).is_none_or(|end| end > *width)
                    || y.checked_add(rect_height).is_none_or(|end| end > *height)
                {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                }
                for row in 0..rect_height {
                    let Some(source_offset) = y
                        .checked_add(row)
                        .and_then(|row| row.checked_mul(*width))
                        .and_then(|pixel| pixel.checked_add(x))
                        .and_then(|pixel| pixel.checked_mul(BYTES_PER_PIXEL))
                        .and_then(|pixel| pixel.checked_add(offset))
                    else {
                        return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                    };
                    let Some(destination_offset) = y
                        .checked_add(row)
                        .and_then(|row| row.checked_mul(*width))
                        .and_then(|pixel| pixel.checked_add(x))
                        .and_then(|pixel| pixel.checked_mul(BYTES_PER_PIXEL))
                    else {
                        return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                    };
                    let count = rect_width * BYTES_PER_PIXEL;
                    if source_offset.checked_add(count).is_none_or(|end| end > *backing_length)
                        || destination_offset.checked_add(count).is_none_or(|end| end > framebuffer.len())
                    {
                        return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                    }
                    let Some(source_address) = backing_address.checked_add(source_offset as u64) else {
                        return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                    };
                    if !memory.read(
                        source_address,
                        &mut framebuffer[destination_offset..destination_offset + count],
                    ) {
                        return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                    }
                }
                Self::write_u32(response, 0, RESP_OK_NODATA);
                HEADER_SIZE
            }
            CMD_RESOURCE_FLUSH => {
                if request.len() < 48
                    || !*resource_created
                    || !*scanout_enabled
                    || Self::read_u32(request, 40) != Some(RESOURCE_ID)
                {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                }
                Self::write_u32(response, 0, RESP_OK_NODATA);
                HEADER_SIZE
            }
            CMD_RESOURCE_DETACH_BACKING => {
                if request.len() < 32
                    || !*resource_created
                    || Self::read_u32(request, 24) != Some(RESOURCE_ID)
                {
                    return Self::error(response, RESP_ERR_INVALID_PARAMETER);
                }
                *backing_address = 0;
                *backing_length = 0;
                Self::write_u32(response, 0, RESP_OK_NODATA);
                HEADER_SIZE
            }
            _ => return Self::error(response, RESP_ERR_UNSPEC),
        };
        response_length
    }

    fn error(response: &mut [u8], response_type: u32) -> usize {
        Self::write_u32(response, 0, response_type);
        HEADER_SIZE
    }

    fn write_rect(response: &mut [u8], offset: usize, x: u32, y: u32, width: u32, height: u32) {
        Self::write_u32(response, offset, x);
        Self::write_u32(response, offset + 4, y);
        Self::write_u32(response, offset + 8, width);
        Self::write_u32(response, offset + 12, height);
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
}
