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

pub(crate) enum ScsiData<'a> {
    None,
    ToDevice(&'a [u8]),
    FromDevice(&'a mut [u8]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ScsiResponse {
    pub(crate) status: u8,
    pub(crate) response: u8,
    pub(crate) status_qualifier: u16,
    pub(crate) residual: u32,
    pub(crate) sense_length: usize,
    pub(crate) sense: [u8; VirtioScsi::SENSE_SIZE],
}

pub(crate) struct VirtioScsi {
    transport: VirtioTransport,
    _control_queue: DmaQueue<{ Self::QUEUE_SIZE }>,
    _event_queue: DmaQueue<{ Self::QUEUE_SIZE }>,
    request_queue: DmaQueue<{ Self::QUEUE_SIZE }>,
    request_buffer: DmaBuffer,
    response_buffer: DmaBuffer,
    data_buffer: DmaBuffer,
    cdb_size: usize,
    sense_size: usize,
    max_target: u8,
    max_lun: u16,
    next_tag: u64,
    in_flight_head: Option<u16>,
}

impl VirtioScsi {
    const DEVICE_ID: u32 = 8;
    const QUEUE_SIZE: usize = 8;
    const REQUEST_HEADER_SIZE: usize = 19;
    const RESPONSE_HEADER_SIZE: usize = 12;
    const SENSE_SIZE: usize = 96;
    const MAX_CDB_SIZE: usize = 32;
    const MAX_TRANSFER_SIZE: usize = 32 * 1024;

    pub(crate) fn initialize() -> Option<Self> {
        let candidate = VirtioTransport::find(Self::DEVICE_ID)?;
        let transport = candidate.initialize(Self::DEVICE_ID).ok()?;
        let request_queue_count = transport.read_device_config(0)?;
        let sense_size = transport.read_device_config(20)? as usize;
        let cdb_size = transport.read_device_config(24)? as usize;
        let max_target_lun = transport.read_device_config(28)?;
        let max_lun = transport.read_device_config(32)?;
        if request_queue_count == 0
            || cdb_size == 0
            || cdb_size > Self::MAX_CDB_SIZE
            || sense_size > Self::SENSE_SIZE
        {
            return None;
        }

        let mut control_queue = DmaQueue::new()?;
        let mut event_queue = DmaQueue::new()?;
        let mut request_queue = DmaQueue::new()?;
        control_queue.configure(&transport, 0).ok()?;
        event_queue.configure(&transport, 1).ok()?;
        request_queue.configure(&transport, 2).ok()?;
        if !transport.activate() {
            return None;
        }

        Some(Self {
            transport,
            _control_queue: control_queue,
            _event_queue: event_queue,
            request_queue,
            request_buffer: DmaArena::allocate(Self::REQUEST_HEADER_SIZE + cdb_size, 16)?,
            response_buffer: DmaArena::allocate(Self::RESPONSE_HEADER_SIZE + Self::SENSE_SIZE, 16)?,
            data_buffer: DmaArena::allocate(Self::MAX_TRANSFER_SIZE, 16)?,
            cdb_size,
            sense_size,
            max_target: ((max_target_lun >> 16) as u16).min(u8::MAX as u16) as u8,
            max_lun: max_lun.min(0x3fff) as u16,
            next_tag: 1,
            in_flight_head: None,
        })
    }

    pub(crate) fn submit<'a>(
        &'a mut self,
        target: u8,
        lun: u16,
        cdb: &[u8],
        data: ScsiData<'a>,
    ) -> Result<ScsiTransfer<'a>, Error> {
        self.reap_abandoned_request()?;
        if target > self.max_target
            || lun > self.max_lun
            || cdb.is_empty()
            || cdb.len() > self.cdb_size
        {
            return Err(Error::Inval);
        }

        let (data_length, device_writes, output) = match data {
            ScsiData::None => (0, false, None),
            ScsiData::ToDevice(input) => {
                if input.len() > Self::MAX_TRANSFER_SIZE {
                    return Err(Error::MsgSize);
                }
                self.data_buffer.as_mut_slice()[..input.len()].copy_from_slice(input);
                (input.len(), false, None)
            }
            ScsiData::FromDevice(output) => {
                if output.len() > Self::MAX_TRANSFER_SIZE {
                    return Err(Error::MsgSize);
                }
                self.data_buffer.as_mut_slice()[..output.len()].fill(0);
                (output.len(), true, Some(output))
            }
        };

        let request = self.request_buffer.as_mut_slice();
        request.fill(0);
        request[0] = 1;
        request[1] = target;
        request[4..6].copy_from_slice(&lun.to_be_bytes());
        request[8..16].copy_from_slice(&self.next_tag.to_le_bytes());
        request[Self::REQUEST_HEADER_SIZE..Self::REQUEST_HEADER_SIZE + cdb.len()]
            .copy_from_slice(cdb);
        self.next_tag = self.next_tag.wrapping_add(1);

        let response_address = self.response_buffer.bus_address();
        let response_length = self.response_buffer.len() as u32;
        let response = self.response_buffer.as_mut_slice();
        response.fill(0);
        let request_descriptor = Descriptor::new(
            self.request_buffer.bus_address(),
            (Self::REQUEST_HEADER_SIZE + self.cdb_size) as u32,
            false,
        );
        let response_descriptor = Descriptor::new(response_address, response_length, true);
        let head = if data_length == 0 {
            self.request_queue
                .submit(&[request_descriptor, response_descriptor])
        } else {
            let data_descriptor = Descriptor::new(
                self.data_buffer.bus_address(),
                data_length as u32,
                device_writes,
            );
            self.request_queue
                .submit(&[request_descriptor, data_descriptor, response_descriptor])
        }
        .ok_or(Error::Busy)?;
        self.in_flight_head = Some(head);
        DmaArena::sync_for_device();
        self.transport.notify(2);
        Ok(ScsiTransfer {
            device: self,
            output,
            data_length,
            completed: None,
        })
    }

    fn reap_abandoned_request(&mut self) -> Result<(), Error> {
        let Some(expected) = self.in_flight_head else {
            return Ok(());
        };
        let Some(used) = self.request_queue.poll_used() else {
            return Err(Error::Busy);
        };
        self.in_flight_head = None;
        if used.id != u32::from(expected) {
            return Err(Error::Proto);
        }
        Ok(())
    }
}

pub(crate) struct ScsiTransfer<'a> {
    device: &'a mut VirtioScsi,
    output: Option<&'a mut [u8]>,
    data_length: usize,
    completed: Option<ScsiResponse>,
}

impl ScsiTransfer<'_> {
    pub(crate) fn poll(&mut self) -> Result<Option<ScsiResponse>, Error> {
        if self.completed.is_some() {
            return Ok(self.completed);
        }
        let Some(used) = self.device.request_queue.poll_used() else {
            return Ok(None);
        };
        let expected = self.device.in_flight_head.take().ok_or(Error::Proto)?;
        if used.id != u32::from(expected) || used.length < VirtioScsi::RESPONSE_HEADER_SIZE as u32 {
            return Err(Error::Proto);
        }

        DmaArena::sync_for_cpu();
        let response_buffer = self.device.response_buffer.as_mut_slice();
        let sense_length = Self::read_u32(response_buffer, 0)? as usize;
        let residual = Self::read_u32(response_buffer, 4)?;
        let status_qualifier = u16::from_le_bytes(
            response_buffer
                .get(8..10)
                .ok_or(Error::Proto)?
                .try_into()
                .map_err(|_| Error::Proto)?,
        );
        let status = response_buffer[10];
        let response_code = response_buffer[11];
        if sense_length > self.device.sense_size
            || sense_length > VirtioScsi::SENSE_SIZE
            || residual as usize > self.data_length
            || (used.length as usize) < VirtioScsi::RESPONSE_HEADER_SIZE + sense_length
        {
            return Err(Error::Proto);
        }

        let mut sense = [0; VirtioScsi::SENSE_SIZE];
        sense[..sense_length].copy_from_slice(
            &response_buffer
                [VirtioScsi::RESPONSE_HEADER_SIZE..VirtioScsi::RESPONSE_HEADER_SIZE + sense_length],
        );
        let response = ScsiResponse {
            status,
            response: response_code,
            status_qualifier,
            residual,
            sense_length,
            sense,
        };
        if response_code == 0 {
            if let Some(output) = self.output.as_mut() {
                let transferred = self.data_length - residual as usize;
                (**output)[..transferred]
                    .copy_from_slice(&self.device.data_buffer.as_mut_slice()[..transferred]);
            }
        }
        self.completed = Some(response);
        Ok(Some(response))
    }

    fn read_u32(buffer: &[u8], offset: usize) -> Result<u32, Error> {
        let bytes = buffer.get(offset..offset + 4).ok_or(Error::Proto)?;
        Ok(u32::from_le_bytes(
            bytes.try_into().map_err(|_| Error::Proto)?,
        ))
    }
}
