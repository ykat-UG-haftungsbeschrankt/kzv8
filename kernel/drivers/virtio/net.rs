/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::memory::dma::{self, DmaArena, DmaBuffer};
use crate::kernel::drivers::virtio::queue::{Descriptor, DmaQueue};
use crate::kernel::drivers::virtio::transport::VirtioTransport;

const DEVICE_ID: u32 = 1;
const QUEUE_SIZE: usize = 8;
const HEADER_SIZE: usize = 12;
const FEATURE_MRG_RXBUF: u64 = 1 << 15;
pub(crate) const MTU: usize = 1500;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Error {
    Unavailable,
    FrameTooLarge,
    QueueBusy,
    BufferTooSmall,
    CompletionMismatch,
}

pub(crate) struct VirtioNet {
    transport: VirtioTransport,
    receive_queue: DmaQueue<QUEUE_SIZE>,
    transmit_queue: DmaQueue<QUEUE_SIZE>,
    receive_buffer: DmaBuffer,
    transmit_buffer: DmaBuffer,
    receive_head: Option<u16>,
    transmit_head: Option<u16>,
    mac_address: [u8; 6],
}

impl VirtioNet {
    pub(crate) fn initialize() -> Option<Self> {
        let candidate = VirtioTransport::find(DEVICE_ID)?;
        let transport = candidate
            .initialize_with_features(
                DEVICE_ID,
                VirtioTransport::net_mac_feature() | FEATURE_MRG_RXBUF,
            )
            .ok()?;
        let mut receive_queue = DmaQueue::new()?;
        let mut transmit_queue = DmaQueue::new()?;
        receive_queue.configure(&transport, 0).ok()?;
        transmit_queue.configure(&transport, 1).ok()?;
        if !transport.activate() {
            return None;
        }
        let receive_buffer = DmaArena::allocate(HEADER_SIZE + MTU, 16)?;
        let transmit_buffer = DmaArena::allocate(HEADER_SIZE + MTU, 16)?;
        let mac_low = transport.read_device_config(0)?;
        let mac_high = transport.read_device_config(4)?;
        let mac_address = [
            mac_low as u8,
            (mac_low >> 8) as u8,
            (mac_low >> 16) as u8,
            (mac_low >> 24) as u8,
            mac_high as u8,
            (mac_high >> 8) as u8,
        ];
        Some(Self {
            transport,
            receive_queue,
            transmit_queue,
            receive_buffer,
            transmit_buffer,
            receive_head: None,
            transmit_head: None,
            mac_address,
        })
    }

    pub(crate) const fn mac_address(&self) -> [u8; 6] {
        self.mac_address
    }

    pub(crate) fn interrupt_slot(&self) -> usize {
        self.transport.slot()
    }

    pub(crate) fn transmit(&mut self, frame: &[u8]) -> Result<(), Error> {
        if frame.len() > MTU {
            return Err(Error::FrameTooLarge);
        }
        if self.transmit_head.is_some() && !self.poll_transmit()? {
            return Err(Error::QueueBusy);
        }
        let buffer = self.transmit_buffer.as_mut_slice();
        buffer[..HEADER_SIZE].fill(0);
        buffer[HEADER_SIZE..HEADER_SIZE + frame.len()].copy_from_slice(frame);
        let base = self.transmit_buffer.bus_address();
        let descriptors = [
            Descriptor::new(base, HEADER_SIZE as u32, false),
            Descriptor::new(base + HEADER_SIZE as u64, frame.len() as u32, false),
        ];
        let head = self
            .transmit_queue
            .submit(&descriptors)
            .ok_or(Error::QueueBusy)?;
        self.transmit_head = Some(head);
        dma::sync_for_device();
        self.transport.notify(1);
        Ok(())
    }

    pub(crate) fn poll_transmit(&mut self) -> Result<bool, Error> {
        let Some(expected) = self.transmit_head else {
            return Ok(true);
        };
        let Some(used) = self.transmit_queue.poll_used() else {
            return Ok(false);
        };
        self.transmit_head = None;
        if used.id != u32::from(expected) {
            return Err(Error::CompletionMismatch);
        }
        Ok(true)
    }

    pub(crate) fn poll_receive(&mut self, output: &mut [u8]) -> Result<Option<usize>, Error> {
        if output.is_empty() {
            return Err(Error::BufferTooSmall);
        }
        if self.receive_head.is_none() {
            let buffer = self.receive_buffer.as_mut_slice();
            buffer.fill(0);
            let base = self.receive_buffer.bus_address();
            let descriptors = [
                Descriptor::new(base, HEADER_SIZE as u32, true),
                Descriptor::new(base + HEADER_SIZE as u64, MTU as u32, true),
            ];
            let head = self
                .receive_queue
                .submit(&descriptors)
                .ok_or(Error::QueueBusy)?;
            dma::sync_for_device();
            self.transport.notify(0);
            self.receive_head = Some(head);
        }

        let head = self.receive_head.unwrap();
        let Some(used) = self.receive_queue.poll_used() else {
            return Ok(None);
        };
        self.receive_head = None;
        if used.id != u32::from(head) {
            return Err(Error::CompletionMismatch);
        }
        let frame_length = (used.length as usize).saturating_sub(HEADER_SIZE);
        if frame_length > output.len() || frame_length > MTU {
            return Err(Error::BufferTooSmall);
        }
        let buffer = self.receive_buffer.as_mut_slice();
        output[..frame_length].copy_from_slice(&buffer[HEADER_SIZE..HEADER_SIZE + frame_length]);
        Ok(Some(frame_length))
    }
}
