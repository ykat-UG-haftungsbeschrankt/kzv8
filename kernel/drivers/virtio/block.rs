/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::drivers::virtio::queue::{Descriptor, DmaQueue};
use crate::kernel::drivers::virtio::transport::VirtioTransport;
use crate::kernel::error::Error;
use crate::kernel::memory::dma::{self, DmaArena, DmaBuffer};

const DEVICE_ID: u32 = 2;
const QUEUE_SIZE: usize = 8;
const SECTOR_SIZE: usize = 512;
const MAX_TRANSFER_SIZE: usize = 64 * 1024;

#[repr(C)]
struct RequestHeader {
    request_type: u32,
    reserved: u32,
    sector: u64,
}

pub(crate) struct BlockDevice {
    transport: VirtioTransport,
    queue: DmaQueue<QUEUE_SIZE>,
    capacity_sectors: u64,
    request_header: RequestHeader,
    request_status: u8,
    dma_buffer: DmaBuffer,
    in_flight_head: Option<u16>,
}

impl BlockDevice {
    pub(crate) fn initialize() -> Option<Self> {
        let candidate = VirtioTransport::find(DEVICE_ID)?;
        let transport = candidate.initialize(DEVICE_ID).ok()?;
        let mut queue = DmaQueue::new()?;
        queue.configure(&transport, 0).ok()?;
        if !transport.activate() {
            return None;
        }
        let dma_buffer = DmaArena::allocate(MAX_TRANSFER_SIZE, 16)?;
        let capacity_sectors = u64::from(transport.read_device_config(0)?)
            | (u64::from(transport.read_device_config(4)?) << 32);
        Some(Self {
            transport,
            queue,
            capacity_sectors,
            request_header: RequestHeader {
                request_type: 0,
                reserved: 0,
                sector: 0,
            },
            request_status: 0xff,
            dma_buffer,
            in_flight_head: None,
        })
    }

    pub(crate) const fn capacity_sectors(&self) -> u64 {
        self.capacity_sectors
    }

    pub(crate) fn submit_read<'a>(
        &'a mut self,
        sector: u64,
        output: &'a mut [u8],
    ) -> Result<BlockTransfer<'a>, Error> {
        self.validate_request(sector, output.len())?;
        self.submit(0, sector, output.len(), Some(output), None)
    }

    pub(crate) fn submit_write<'a>(
        &'a mut self,
        sector: u64,
        input: &[u8],
    ) -> Result<BlockTransfer<'a>, Error> {
        self.validate_request(sector, input.len())?;
        self.submit(1, sector, input.len(), None, Some(input))
    }

    fn validate_request(&self, sector: u64, length: usize) -> Result<(), Error> {
        if length == 0 || length % SECTOR_SIZE != 0 || length > MAX_TRANSFER_SIZE {
            return Err(Error::Inval);
        }
        if sector
            .checked_add((length / SECTOR_SIZE) as u64)
            .is_none_or(|end| end > self.capacity_sectors)
        {
            return Err(Error::Inval);
        }
        Ok(())
    }

    fn submit<'a>(
        &'a mut self,
        request_type: u32,
        sector: u64,
        length: usize,
        output: Option<&'a mut [u8]>,
        input: Option<&[u8]>,
    ) -> Result<BlockTransfer<'a>, Error> {
        self.reap_abandoned_request()?;
        if let Some(input) = input {
            self.dma_buffer.as_mut_slice()[..length].copy_from_slice(input);
        }
        self.request_header = RequestHeader {
            request_type,
            reserved: 0,
            sector,
        };
        self.request_status = 0xff;
        let descriptors = [
            Descriptor::new(
                &self.request_header as *const _ as u64,
                core::mem::size_of::<RequestHeader>() as u32,
                false,
            ),
            Descriptor::new(
                self.dma_buffer.bus_address(),
                length as u32,
                request_type == 0,
            ),
            Descriptor::new(&mut self.request_status as *mut u8 as u64, 1, true),
        ];
        let head = self.queue.submit(&descriptors).ok_or(Error::Busy)?;
        self.in_flight_head = Some(head);
        crate::kernel::memory::dma::sync_for_device();
        self.transport.notify(0);
        Ok(BlockTransfer {
            device: self,
            output,
            length,
            head,
            complete: false,
        })
    }

    fn reap_abandoned_request(&mut self) -> Result<(), Error> {
        let Some(expected) = self.in_flight_head else {
            return Ok(());
        };
        let Some(used) = self.queue.poll_used() else {
            return Err(Error::Busy);
        };
        self.in_flight_head = None;
        dma::sync_for_cpu();
        if used.id != u32::from(expected) {
            return Err(Error::Proto);
        }
        Ok(())
    }

    pub(crate) fn interrupt_slot(&self) -> usize {
        self.transport.slot()
    }
}

pub(crate) struct BlockTransfer<'a> {
    device: &'a mut BlockDevice,
    output: Option<&'a mut [u8]>,
    length: usize,
    head: u16,
    complete: bool,
}

impl BlockTransfer<'_> {
    pub(crate) fn poll(&mut self) -> Result<bool, Error> {
        if self.complete {
            return Ok(true);
        }
        let Some(used) = self.device.queue.poll_used() else {
            return Ok(false);
        };
        self.device.in_flight_head = None;
        dma::sync_for_cpu();
        self.complete = true;
        if used.id != u32::from(self.head) {
            return Err(Error::Proto);
        }
        match self.device.request_status {
            0 => {}
            1 => return Err(Error::Io),
            2 => return Err(Error::OpNotSupp),
            _ => return Err(Error::Proto),
        }
        if let Some(output) = self.output.as_mut() {
            let source = &self.device.dma_buffer.as_mut_slice()[..self.length];
            (**output).copy_from_slice(source);
        }
        Ok(true)
    }
}
