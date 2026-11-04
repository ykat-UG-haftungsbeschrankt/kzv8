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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RtcClockCapabilities {
    pub(crate) clock_type: u8,
    pub(crate) leap_second_smearing: u8,
    pub(crate) flags: u8,
}

pub(crate) struct VirtioRtc {
    transport: VirtioTransport,
    request_queue: DmaQueue<{ Self::QUEUE_SIZE }>,
    message_buffer: DmaBuffer,
    num_clocks: u16,
}

impl VirtioRtc {
    const DEVICE_ID: u32 = 42;
    const QUEUE_SIZE: usize = 8;
    const MESSAGE_BUFFER_SIZE: usize = 128;
    const REQUEST_SIZE: usize = 64;
    const RESPONSE_OFFSET: usize = Self::REQUEST_SIZE;
    const RESPONSE_SIZE: usize = Self::MESSAGE_BUFFER_SIZE - Self::RESPONSE_OFFSET;
    const MESSAGE_HEADER_SIZE: usize = 8;
    const REQUEST_SPIN_LIMIT: usize = 1_000_000;

    const REQ_READ: u16 = 0x0001;
    const REQ_READ_CROSS: u16 = 0x0002;
    const REQ_CFG: u16 = 0x1000;
    const REQ_CLOCK_CAP: u16 = 0x1001;
    const REQ_CROSS_CAP: u16 = 0x1002;
    const RESP_OK: u8 = 0;
    const RESP_NOT_SUPPORTED: u8 = 2;
    const RESP_NO_DEVICE: u8 = 3;
    const RESP_INVALID: u8 = 4;

    pub(crate) fn initialize() -> Option<Self> {
        let candidate = VirtioTransport::find(Self::DEVICE_ID)?;
        let transport = candidate.initialize(Self::DEVICE_ID).ok()?;
        let mut request_queue = DmaQueue::new()?;
        let message_buffer = DmaArena::allocate(Self::MESSAGE_BUFFER_SIZE, 16)?;
        request_queue
            .configure_with_min_size(&transport, 0, 2)
            .ok()?;
        if !transport.activate() {
            return None;
        }

        let mut rtc = Self {
            transport,
            request_queue,
            message_buffer,
            num_clocks: 0,
        };
        let mut request = [0; Self::MESSAGE_HEADER_SIZE];
        Self::write_message_type(&mut request, Self::REQ_CFG);
        let mut response = [0; 16];
        rtc.exchange(&request, &mut response).ok()?;
        rtc.num_clocks = Self::read_u16(&response, Self::MESSAGE_HEADER_SIZE).ok()?;
        (rtc.num_clocks != 0).then_some(rtc)
    }

    pub(crate) const fn num_clocks(&self) -> u16 {
        self.num_clocks
    }

    pub(crate) fn clock_capabilities(
        &mut self,
        clock_id: u16,
    ) -> Result<RtcClockCapabilities, Error> {
        self.validate_clock(clock_id)?;
        let mut request = [0; 16];
        Self::write_message_type(&mut request, Self::REQ_CLOCK_CAP);
        Self::write_u16(&mut request, Self::MESSAGE_HEADER_SIZE, clock_id);
        let mut response = [0; 16];
        self.exchange(&request, &mut response)?;
        Ok(RtcClockCapabilities {
            clock_type: response[Self::MESSAGE_HEADER_SIZE],
            leap_second_smearing: response[Self::MESSAGE_HEADER_SIZE + 1],
            flags: response[Self::MESSAGE_HEADER_SIZE + 2],
        })
    }

    pub(crate) fn read(&mut self, clock_id: u16) -> Result<u64, Error> {
        self.validate_clock(clock_id)?;
        let mut request = [0; 16];
        Self::write_message_type(&mut request, Self::REQ_READ);
        Self::write_u16(&mut request, Self::MESSAGE_HEADER_SIZE, clock_id);
        let mut response = [0; 16];
        self.exchange(&request, &mut response)?;
        Self::read_u64(&response, Self::MESSAGE_HEADER_SIZE)
    }

    pub(crate) fn read_cross(
        &mut self,
        clock_id: u16,
        hardware_counter: u8,
    ) -> Result<(u64, u64), Error> {
        self.validate_clock(clock_id)?;
        if hardware_counter > 1 {
            return Err(Error::Inval);
        }
        let mut request = [0; 16];
        Self::write_message_type(&mut request, Self::REQ_READ_CROSS);
        Self::write_u16(&mut request, Self::MESSAGE_HEADER_SIZE, clock_id);
        request[Self::MESSAGE_HEADER_SIZE + 2] = hardware_counter;
        let mut response = [0; 24];
        self.exchange(&request, &mut response)?;
        Ok((
            Self::read_u64(&response, Self::MESSAGE_HEADER_SIZE)?,
            Self::read_u64(&response, Self::MESSAGE_HEADER_SIZE + 8)?,
        ))
    }

    fn validate_clock(&self, clock_id: u16) -> Result<(), Error> {
        if clock_id >= self.num_clocks {
            return Err(Error::Inval);
        }
        Ok(())
    }

    fn exchange(&mut self, request: &[u8], response: &mut [u8]) -> Result<(), Error> {
        if request.len() > Self::REQUEST_SIZE || response.len() > Self::RESPONSE_SIZE {
            return Err(Error::MsgSize);
        }
        let base = self.message_buffer.bus_address();
        let buffer = self.message_buffer.as_mut_slice();
        buffer[..Self::REQUEST_SIZE].fill(0);
        buffer[..request.len()].copy_from_slice(request);
        buffer[Self::RESPONSE_OFFSET..].fill(0);
        let descriptors = [
            Descriptor::new(base, request.len() as u32, false),
            Descriptor::new(
                base + Self::RESPONSE_OFFSET as u64,
                response.len() as u32,
                true,
            ),
        ];
        let head = self.request_queue.submit(&descriptors).ok_or(Error::Busy)?;
        DmaArena::sync_for_device();
        self.transport.notify(0);

        for _ in 0..Self::REQUEST_SPIN_LIMIT {
            if let Some(used) = self.request_queue.poll_used() {
                if used.id != u32::from(head) || used.length < response.len() as u32 {
                    return Err(Error::Proto);
                }
                let response_bytes =
                    &buffer[Self::RESPONSE_OFFSET..Self::RESPONSE_OFFSET + response.len()];
                match response_bytes[0] {
                    Self::RESP_OK => {}
                    Self::RESP_NOT_SUPPORTED => return Err(Error::OpNotSupp),
                    Self::RESP_NO_DEVICE => return Err(Error::NoDev),
                    Self::RESP_INVALID => return Err(Error::Inval),
                    _ => return Err(Error::Io),
                }
                response.copy_from_slice(response_bytes);
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(Error::Io)
    }

    fn write_message_type(buffer: &mut [u8], message_type: u16) {
        Self::write_u16(buffer, 0, message_type);
    }

    fn write_u16(buffer: &mut [u8], offset: usize, value: u16) {
        buffer[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn read_u16(buffer: &[u8], offset: usize) -> Result<u16, Error> {
        let bytes = buffer.get(offset..offset + 2).ok_or(Error::Proto)?;
        Ok(u16::from_le_bytes(
            bytes.try_into().map_err(|_| Error::Proto)?,
        ))
    }

    fn read_u64(buffer: &[u8], offset: usize) -> Result<u64, Error> {
        let bytes = buffer.get(offset..offset + 8).ok_or(Error::Proto)?;
        Ok(u64::from_le_bytes(
            bytes.try_into().map_err(|_| Error::Proto)?,
        ))
    }
}
