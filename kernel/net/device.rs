/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use crate::kernel::driver::virtio::net::VirtioNet;
use crate::kernel::error::Error;

pub(crate) struct EthernetDevice {
    device: VirtioNet,
    receive_frame: Vec<u8>,
}

impl EthernetDevice {
    pub(crate) fn initialize() -> Option<Self> {
        let device = VirtioNet::initialize()?;
        let frame_size = device.max_frame_size();
        let mut receive_frame = Vec::new();
        receive_frame.try_reserve_exact(frame_size).ok()?;
        receive_frame.resize(frame_size, 0);
        Some(Self {
            device,
            receive_frame,
        })
    }

    pub(crate) const fn mac_address(&self) -> [u8; 6] {
        self.device.mac_address()
    }

    pub(crate) const fn max_frame_size(&self) -> usize {
        self.device.max_frame_size()
    }

    pub(crate) const fn mtu(&self) -> usize {
        self.device.mtu()
    }

    pub(crate) fn poll_receive_frame(&mut self) -> Result<Option<usize>, Error> {
        self.device.poll_receive(self.receive_frame.as_mut_slice())
    }

    pub(crate) fn received_frame(&self, length: usize) -> Option<&[u8]> {
        self.receive_frame.get(..length)
    }

    pub(crate) fn transmit_frame(&mut self, frame: &[u8]) -> Result<(), Error> {
        self.device.transmit(frame)
    }

    #[allow(dead_code)]
    pub(crate) fn interrupt_slot(&self) -> usize {
        self.device.interrupt_slot()
    }
}
