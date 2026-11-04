/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use smoltcp::phy::{
    Device, DeviceCapabilities, Medium, RxToken as SmolRxToken, TxToken as SmolTxToken,
};
use smoltcp::time::Instant;
use crate::kernel::driver::virtio::net::VirtioNet;

pub(crate) struct EthernetDevice {
    device: VirtioNet,
    receive_frame: Vec<u8>,
    transmit_frame: Vec<u8>,
}

impl EthernetDevice {
    pub(crate) fn initialize() -> Option<Self> {
        let device = VirtioNet::initialize()?;
        let frame_size = device.max_frame_size();
        let mut receive_frame = Vec::new();
        receive_frame.try_reserve_exact(frame_size).ok()?;
        receive_frame.resize(frame_size, 0);
        let mut transmit_frame = Vec::new();
        transmit_frame.try_reserve_exact(frame_size).ok()?;
        transmit_frame.resize(frame_size, 0);
        Some(Self {
            device,
            receive_frame,
            transmit_frame,
        })
    }

    pub(crate) const fn mac_address(&self) -> [u8; 6] {
        self.device.mac_address()
    }

    pub(crate) fn interrupt_slot(&self) -> usize {
        self.device.interrupt_slot()
    }
}

pub(crate) struct RxToken<'a> {
    frame: &'a [u8],
}

impl SmolRxToken for RxToken<'_> {
    fn consume<R, F>(self, function: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        function(self.frame)
    }
}

pub(crate) struct TxToken<'a> {
    device: &'a mut VirtioNet,
    frame: &'a mut [u8],
}

impl SmolTxToken for TxToken<'_> {
    fn consume<R, F>(self, length: usize, function: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let length = length.min(self.frame.len());
        let result = function(&mut self.frame[..length]);
        let _ = self.device.transmit(&self.frame[..length]);
        result
    }
}

impl Device for EthernetDevice {
    type RxToken<'a>
        = RxToken<'a>
    where
        Self: 'a;
    type TxToken<'a>
        = TxToken<'a>
    where
        Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let Self {
            device,
            receive_frame,
            transmit_frame,
        } = self;
        let length = device
            .poll_receive(receive_frame.as_mut_slice())
            .ok()
            .flatten()?;
        Some((
            RxToken {
                frame: &receive_frame[..length],
            },
            TxToken {
                device,
                frame: transmit_frame.as_mut_slice(),
            },
        ))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        let Self {
            device,
            transmit_frame,
            ..
        } = self;
        Some(TxToken {
            device,
            frame: transmit_frame.as_mut_slice(),
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut capabilities = DeviceCapabilities::default();
        capabilities.max_transmission_unit = self.device.mtu();
        capabilities.medium = Medium::Ethernet;
        capabilities
    }
}
