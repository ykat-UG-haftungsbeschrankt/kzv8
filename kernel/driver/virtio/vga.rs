/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use of this file is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::driver::virtio::queue::{Descriptor, DmaQueue};
use crate::kernel::driver::virtio::transport::VirtioTransport;
use crate::kernel::error::Error;
use crate::kernel::memory::dma::{DmaArena, DmaBuffer};

pub(crate) struct VirtioGpu {
    transport: VirtioTransport,
    control_queue: DmaQueue<{ Self::QUEUE_SIZE }>,
    _cursor_queue: DmaQueue<{ Self::QUEUE_SIZE }>,
    control_buffer: DmaBuffer,
    framebuffer: Option<DmaBuffer>,
    width: usize,
    height: usize,
}

impl VirtioGpu {
    const DEVICE_ID: u32 = 16;
    const QUEUE_SIZE: usize = 8;
    const RESOURCE_ID: u32 = 1;
    const FRAMEBUFFER_BUDGET_BYTES: usize = 256 * 1024;
    const BYTES_PER_PIXEL: usize = 4;
    const CONTROL_REQUEST_SIZE: usize = 512;
    const CONTROL_RESPONSE_OFFSET: usize = Self::CONTROL_REQUEST_SIZE;
    const CONTROL_RESPONSE_SIZE: usize = 512;
    const CONTROL_BUFFER_SIZE: usize = Self::CONTROL_REQUEST_SIZE + Self::CONTROL_RESPONSE_SIZE;
    const GPU_HEADER_SIZE: usize = 24;
    const DISPLAY_INFO_SIZE: usize = Self::GPU_HEADER_SIZE + 16 * 24;
    const COMMAND_SPIN_LIMIT: usize = 1_000_000;

    const CMD_GET_DISPLAY_INFO: u32 = 0x0100;
    const CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
    const CMD_SET_SCANOUT: u32 = 0x0103;
    const CMD_RESOURCE_FLUSH: u32 = 0x0104;
    const CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
    const CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
    const RESP_OK_NODATA: u32 = 0x1100;
    const RESP_OK_DISPLAY_INFO: u32 = 0x1101;
    const FORMAT_B8G8R8A8_UNORM: u32 = 1;

    pub(crate) fn initialize() -> Option<Self> {
        let candidate = VirtioTransport::find(Self::DEVICE_ID)?;
        let transport = candidate.initialize(Self::DEVICE_ID).ok()?;
        let mut control_queue = DmaQueue::new()?;
        let mut cursor_queue = DmaQueue::new()?;
        let control_buffer = DmaArena::allocate(Self::CONTROL_BUFFER_SIZE, 16)?;
        control_queue
            .configure_with_min_size(&transport, 0, 2)
            .ok()?;
        cursor_queue.configure(&transport, 1).ok()?;
        if !transport.activate() {
            return None;
        }

        let mut gpu = Self {
            transport,
            control_queue,
            _cursor_queue: cursor_queue,
            control_buffer,
            framebuffer: None,
            width: 0,
            height: 0,
        };
        gpu.configure_scanout().ok()?;
        gpu.clear(0xff00_0000);
        gpu.present().ok()?;
        Some(gpu)
    }

    pub(crate) const fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub(crate) fn clear(&mut self, color: u32) {
        let pixel = color.to_le_bytes();
        let byte_count = self.width * self.height * Self::BYTES_PER_PIXEL;
        let framebuffer = self
            .framebuffer
            .as_mut()
            .expect("GPU framebuffer must be configured before use");
        for bytes in framebuffer.as_mut_slice()[..byte_count].chunks_exact_mut(4) {
            bytes.copy_from_slice(&pixel);
        }
    }

    pub(crate) fn set_pixel(&mut self, x: usize, y: usize, color: u32) -> Result<(), Error> {
        if x >= self.width || y >= self.height {
            return Err(Error::Inval);
        }
        let offset = (y * self.width + x) * 4;
        let framebuffer = self.framebuffer.as_mut().ok_or(Error::NoDev)?;
        framebuffer.as_mut_slice()[offset..offset + 4].copy_from_slice(&color.to_le_bytes());
        Ok(())
    }

    pub(crate) fn present(&mut self) -> Result<(), Error> {
        self.transfer_to_host()?;
        self.flush()
    }

    fn configure_scanout(&mut self) -> Result<(), Error> {
        let mut request = [0; Self::GPU_HEADER_SIZE];
        Self::write_header(&mut request, Self::CMD_GET_DISPLAY_INFO);
        let response_length = self.send_command(
            &request,
            Self::DISPLAY_INFO_SIZE,
            Self::RESP_OK_DISPLAY_INFO,
        )?;
        if response_length < Self::DISPLAY_INFO_SIZE {
            return Err(Error::Proto);
        }

        let buffer = self.control_buffer.as_mut_slice();
        let display = Self::CONTROL_RESPONSE_OFFSET + Self::GPU_HEADER_SIZE;
        let enabled = Self::read_u32(buffer, display + 16)? != 0;
        if !enabled {
            return Err(Error::NoDev);
        }
        let display_width = Self::read_u32(buffer, display + 8)?;
        let display_height = Self::read_u32(buffer, display + 12)?;
        (self.width, self.height) = Self::fit_dimensions(display_width, display_height)?;
        let framebuffer_length = self
            .width
            .checked_mul(self.height)
            .and_then(|pixels| pixels.checked_mul(Self::BYTES_PER_PIXEL))
            .filter(|length| *length <= Self::FRAMEBUFFER_BUDGET_BYTES)
            .ok_or(Error::Overflow)?;
        let mut framebuffer =
            DmaArena::allocate(framebuffer_length, 16).ok_or(Error::NoMem)?;
        framebuffer.as_mut_slice().fill(0);
        let framebuffer_address = framebuffer.bus_address();
        self.framebuffer = Some(framebuffer);

        let mut create = [0; 40];
        Self::write_header(&mut create, Self::CMD_RESOURCE_CREATE_2D);
        Self::write_u32(&mut create, 24, Self::RESOURCE_ID);
        Self::write_u32(&mut create, 28, Self::FORMAT_B8G8R8A8_UNORM);
        Self::write_u32(&mut create, 32, self.width as u32);
        Self::write_u32(&mut create, 36, self.height as u32);
        self.send_no_data(&create)?;

        let mut attach = [0; 48];
        Self::write_header(&mut attach, Self::CMD_RESOURCE_ATTACH_BACKING);
        Self::write_u32(&mut attach, 24, Self::RESOURCE_ID);
        Self::write_u32(&mut attach, 28, 1);
        Self::write_u64(&mut attach, 32, framebuffer_address);
        Self::write_u32(&mut attach, 40, framebuffer_length as u32);
        self.send_no_data(&attach)?;

        let mut set_scanout = [0; 48];
        Self::write_header(&mut set_scanout, Self::CMD_SET_SCANOUT);
        Self::write_rect(&mut set_scanout, 24, self.width as u32, self.height as u32);
        Self::write_u32(&mut set_scanout, 40, 0);
        Self::write_u32(&mut set_scanout, 44, Self::RESOURCE_ID);
        self.send_no_data(&set_scanout)
    }

    fn fit_dimensions(width: u32, height: u32) -> Result<(usize, usize), Error> {
        if width == 0 || height == 0 {
            return Err(Error::Inval);
        }
        let width = u64::from(width);
        let height = u64::from(height);
        let maximum_pixels = (Self::FRAMEBUFFER_BUDGET_BYTES / Self::BYTES_PER_PIXEL) as u64;
        let pixels = width.checked_mul(height).ok_or(Error::Overflow)?;
        if pixels <= maximum_pixels {
            return Ok((
                usize::try_from(width).map_err(|_| Error::Overflow)?,
                usize::try_from(height).map_err(|_| Error::Overflow)?,
            ));
        }

        let scale_denominator = width.max(height);
        let mut low = 1u64;
        let mut high = scale_denominator;
        let mut fitted_width = 0u64;
        let mut fitted_height = 0u64;
        while low <= high {
            let scale = low + (high - low) / 2;
            let candidate_width = (width * scale / scale_denominator).max(1);
            let candidate_height = (height * scale / scale_denominator).max(1);
            if candidate_width * candidate_height <= maximum_pixels {
                fitted_width = candidate_width;
                fitted_height = candidate_height;
                low = scale + 1;
            } else {
                high = scale - 1;
            }
        }
        Ok((
            usize::try_from(fitted_width).map_err(|_| Error::Overflow)?,
            usize::try_from(fitted_height).map_err(|_| Error::Overflow)?,
        ))
    }

    fn transfer_to_host(&mut self) -> Result<(), Error> {
        let mut request = [0; 56];
        Self::write_header(&mut request, Self::CMD_TRANSFER_TO_HOST_2D);
        Self::write_rect(&mut request, 24, self.width as u32, self.height as u32);
        Self::write_u64(&mut request, 40, 0);
        Self::write_u32(&mut request, 48, Self::RESOURCE_ID);
        self.send_no_data(&request)
    }

    fn flush(&mut self) -> Result<(), Error> {
        let mut request = [0; 48];
        Self::write_header(&mut request, Self::CMD_RESOURCE_FLUSH);
        Self::write_rect(&mut request, 24, self.width as u32, self.height as u32);
        Self::write_u32(&mut request, 40, Self::RESOURCE_ID);
        self.send_no_data(&request)
    }

    fn send_no_data(&mut self, request: &[u8]) -> Result<(), Error> {
        self.send_command(request, Self::GPU_HEADER_SIZE, Self::RESP_OK_NODATA)
            .map(|_| ())
    }

    fn send_command(
        &mut self,
        request: &[u8],
        response_length: usize,
        expected_response: u32,
    ) -> Result<usize, Error> {
        if request.len() > Self::CONTROL_REQUEST_SIZE
            || response_length > Self::CONTROL_RESPONSE_SIZE
        {
            return Err(Error::MsgSize);
        }
        let base = self.control_buffer.bus_address();
        let buffer = self.control_buffer.as_mut_slice();
        buffer[..Self::CONTROL_REQUEST_SIZE].fill(0);
        buffer[..request.len()].copy_from_slice(request);
        let response_range =
            Self::CONTROL_RESPONSE_OFFSET..Self::CONTROL_RESPONSE_OFFSET + response_length;
        buffer[response_range.clone()].fill(0);

        let descriptors = [
            Descriptor::new(base, request.len() as u32, false),
            Descriptor::new(
                base + Self::CONTROL_RESPONSE_OFFSET as u64,
                response_length as u32,
                true,
            ),
        ];
        let head = self.control_queue.submit(&descriptors).ok_or(Error::Busy)?;
        DmaArena::sync_for_device();
        self.transport.notify(0);

        for _ in 0..Self::COMMAND_SPIN_LIMIT {
            if let Some(used) = self.control_queue.poll_used() {
                if used.id != u32::from(head) || used.length < Self::GPU_HEADER_SIZE as u32 {
                    return Err(Error::Proto);
                }
                let response_type = Self::read_u32(buffer, Self::CONTROL_RESPONSE_OFFSET)?;
                if response_type != expected_response {
                    return Err(Error::OpNotSupp);
                }
                return Ok(used.length as usize);
            }
            core::hint::spin_loop();
        }
        Err(Error::Io)
    }

    fn write_header(buffer: &mut [u8], command: u32) {
        Self::write_u32(buffer, 0, command);
    }

    fn write_rect(buffer: &mut [u8], offset: usize, width: u32, height: u32) {
        Self::write_u32(buffer, offset, 0);
        Self::write_u32(buffer, offset + 4, 0);
        Self::write_u32(buffer, offset + 8, width);
        Self::write_u32(buffer, offset + 12, height);
    }

    fn write_u32(buffer: &mut [u8], offset: usize, value: u32) {
        buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u64(buffer: &mut [u8], offset: usize, value: u64) {
        buffer[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn read_u32(buffer: &[u8], offset: usize) -> Result<u32, Error> {
        let bytes = buffer.get(offset..offset + 4).ok_or(Error::Proto)?;
        Ok(u32::from_le_bytes(
            bytes.try_into().map_err(|_| Error::Proto)?,
        ))
    }
}
