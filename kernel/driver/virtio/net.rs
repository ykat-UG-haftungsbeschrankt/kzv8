/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use crate::kernel::driver::virtio::queue::{Descriptor, DmaQueue};
use crate::kernel::driver::virtio::transport::VirtioTransport;
use crate::kernel::error::Error;
use crate::kernel::memory::dma::{DmaArena, DmaBuffer};



pub(crate) struct VirtioNet {
    transport: VirtioTransport,
    receive_queue: DmaQueue<{Self::QUEUE_SIZE}>,
    transmit_queue: DmaQueue<{ Self::QUEUE_SIZE }>,
    receive_buffers: Vec<ReceiveBuffer>,
    transmit_buffers: Vec<TransmitBuffer>,
    mtu: usize,
    frame_size: usize,
    mac_address: [u8; 6],
}

struct ReceiveBuffer {
    buffer: DmaBuffer,
    head: Option<u16>,
    used_length: Option<usize>,
}

struct TransmitBuffer {
    buffer: DmaBuffer,
    head: Option<u16>,
}

impl VirtioNet {
    const DEVICE_ID: u32 = 1;
    const QUEUE_SIZE: usize = 8;
    const HEADER_SIZE: usize = 12;
    const ETHERNET_HEADER_SIZE: usize = 14;
    const MIN_MTU: usize = 68;
    const DEFAULT_MTU: usize = 1500;
    const MTU_CONFIG_OFFSET: usize = 10;
    const FEATURE_MRG_RXBUF: u64 = 1 << 15;

    pub(crate) fn initialize() -> Option<Self> {
        let candidate = VirtioTransport::find(Self::DEVICE_ID)?;
        let (transport, negotiated_features) = candidate
            .initialize_with_optional_features(
                Self::DEVICE_ID,
                VirtioTransport::net_mac_feature() | Self::FEATURE_MRG_RXBUF,
                VirtioTransport::net_mtu_feature(),
            )
            .ok()?;
        let mut receive_queue = DmaQueue::new()?;
        let mut transmit_queue = DmaQueue::new()?;
        receive_queue
            .configure_with_min_size(&transport, 0, 2)
            .ok()?;
        transmit_queue
            .configure_with_min_size(&transport, 1, 2)
            .ok()?;
        if !transport.activate() {
            return None;
        }
        let mtu = if negotiated_features & VirtioTransport::net_mtu_feature() != 0 {
            usize::from(transport.read_device_config_u16(Self::MTU_CONFIG_OFFSET)?)
        } else {
            Self::DEFAULT_MTU
        };
        if mtu < Self::MIN_MTU {
            return None;
        }
        let frame_size = mtu.checked_add(Self::ETHERNET_HEADER_SIZE)?;
        let buffer_size = Self::HEADER_SIZE.checked_add(frame_size)?;
        let request_count = Self::QUEUE_SIZE / 2;
        let mut receive_buffers = Vec::new();
        receive_buffers.try_reserve_exact(request_count).ok()?;
        let mut transmit_buffers = Vec::new();
        transmit_buffers.try_reserve_exact(request_count).ok()?;
        for _ in 0..request_count {
            receive_buffers.push(ReceiveBuffer {
                buffer: DmaArena::allocate(buffer_size, 16)?,
                head: None,
                used_length: None,
            });
            transmit_buffers.push(TransmitBuffer {
                buffer: DmaArena::allocate(buffer_size, 16)?,
                head: None,
            });
        }
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
            receive_buffers,
            transmit_buffers,
            mtu,
            frame_size,
            mac_address,
        })
    }

    pub(crate) const fn mac_address(&self) -> [u8; 6] {
        self.mac_address
    }

    pub(crate) const fn mtu(&self) -> usize {
        self.mtu
    }

    pub(crate) const fn max_frame_size(&self) -> usize {
        self.frame_size
    }

    pub(crate) fn interrupt_slot(&self) -> usize {
        self.transport.slot()
    }

    pub(crate) fn transmit(&mut self, frame: &[u8]) -> Result<(), Error> {
        if frame.len() > self.frame_size {
            return Err(Error::MsgSize);
        }
        self.reap_transmit()?;
        let buffer_index = self
            .transmit_buffers
            .iter()
            .position(|buffer| buffer.head.is_none())
            .ok_or(Error::Busy)?;
        let buffer = self.transmit_buffers[buffer_index].buffer.as_mut_slice();
        buffer[..Self::HEADER_SIZE].fill(0);
        buffer[Self::HEADER_SIZE..Self::HEADER_SIZE + frame.len()].copy_from_slice(frame);
        let base = self.transmit_buffers[buffer_index].buffer.bus_address();
        let descriptors = [
            Descriptor::new(base, Self::HEADER_SIZE as u32, false),
            Descriptor::new(base + Self::HEADER_SIZE as u64, frame.len() as u32, false),
        ];
        let head = self
            .transmit_queue
            .submit(&descriptors)
            .ok_or(Error::Busy)?;
        self.transmit_buffers[buffer_index].head = Some(head);
        DmaArena::sync_for_device();
        self.transport.notify(1);
        Ok(())
    }

    pub(crate) fn poll_transmit(&mut self) -> Result<bool, Error> {
        self.reap_transmit()?;
        Ok(self.transmit_buffers.iter().all(|buffer| buffer.head.is_none()))
    }

    pub(crate) fn poll_receive(&mut self, output: &mut [u8]) -> Result<Option<usize>, Error> {
        if output.is_empty() {
            return Err(Error::MsgSize);
        }
        self.submit_receive_buffers();
        self.reap_receive()?;
        let Some(buffer_index) = self
            .receive_buffers
            .iter()
            .position(|buffer| buffer.used_length.is_some())
        else {
            return Ok(None);
        };
        let used_length = self.receive_buffers[buffer_index]
            .used_length
            .take()
            .ok_or(Error::Proto)?;
        let frame_length = used_length.saturating_sub(Self::HEADER_SIZE);
        if frame_length > output.len() || frame_length > self.frame_size {
            self.submit_receive_buffers();
            return Err(Error::MsgSize);
        }
        let buffer = self.receive_buffers[buffer_index].buffer.as_mut_slice();
        output[..frame_length].copy_from_slice(&buffer[Self::HEADER_SIZE..Self::HEADER_SIZE + frame_length]);
        self.submit_receive_buffers();
        Ok(Some(frame_length))
    }

    fn reap_transmit(&mut self) -> Result<(), Error> {
        while let Some(used) = self.transmit_queue.poll_used() {
            let Some(buffer) = self
                .transmit_buffers
                .iter_mut()
                .find(|buffer| buffer.head.is_some_and(|head| u32::from(head) == used.id))
            else {
                return Err(Error::Proto);
            };
            buffer.head = None;
        }
        Ok(())
    }

    fn reap_receive(&mut self) -> Result<(), Error> {
        while let Some(used) = self.receive_queue.poll_used() {
            let Some(buffer) = self
                .receive_buffers
                .iter_mut()
                .find(|buffer| buffer.head.is_some_and(|head| u32::from(head) == used.id))
            else {
                return Err(Error::Proto);
            };
            buffer.head = None;
            buffer.used_length = Some(used.length as usize);
        }
        Ok(())
    }

    fn submit_receive_buffers(&mut self) {
        let mut submitted = false;
        for index in 0..self.receive_buffers.len() {
            let receive_buffer = &mut self.receive_buffers[index];
            if receive_buffer.head.is_some() || receive_buffer.used_length.is_some() {
                continue;
            }
            let buffer = receive_buffer.buffer.as_mut_slice();
            buffer.fill(0);
            let base = receive_buffer.buffer.bus_address();
            let descriptors = [
                Descriptor::new(base, Self::HEADER_SIZE as u32, true),
                Descriptor::new(base + Self::HEADER_SIZE as u64, self.frame_size as u32, true),
            ];
            let Some(head) = self.receive_queue.submit(&descriptors) else {
                break;
            };
            self.receive_buffers[index].head = Some(head);
            submitted = true;
        }
        if submitted {
            DmaArena::sync_for_device();
            self.transport.notify(0);
        }
    }
}
