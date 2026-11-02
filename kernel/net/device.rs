/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use smoltcp::phy::{
    Device, DeviceCapabilities, Medium, RxToken as SmolRxToken, TxToken as SmolTxToken,
};
use smoltcp::time::Instant;

use crate::kernel::drivers::virtio::net::{self, VirtioNet};

pub(crate) struct EthernetDevice {
    device: VirtioNet,
}

impl EthernetDevice {
    pub(crate) fn initialize() -> Option<Self> {
        Some(Self {
            device: VirtioNet::initialize()?,
        })
    }

    pub(crate) const fn mac_address(&self) -> [u8; 6] {
        self.device.mac_address()
    }

    pub(crate) fn interrupt_slot(&self) -> usize {
        self.device.interrupt_slot()
    }
}

pub(crate) struct RxToken {
    frame: [u8; virtio_net::MTU],
    length: usize,
}

impl SmolRxToken for RxToken {
    fn consume<R, F>(self, function: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        function(&self.frame[..self.length])
    }
}

pub(crate) struct TxToken<'a> {
    device: &'a mut VirtioNet,
}

impl SmolTxToken for TxToken<'_> {
    fn consume<R, F>(self, length: usize, function: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut frame = [0u8; virtio_net::MTU];
        let length = length.min(frame.len());
        let result = function(&mut frame[..length]);
        let _ = self.device.transmit(&frame[..length]);
        result
    }
}

impl Device for EthernetDevice {
    type RxToken<'a>
        = RxToken
    where
        Self: 'a;
    type TxToken<'a>
        = TxToken<'a>
    where
        Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let mut frame = [0u8; virtio_net::MTU];
        let length = self.device.poll_receive(&mut frame).ok().flatten()?;
        Some((
            RxToken { frame, length },
            TxToken {
                device: &mut self.device,
            },
        ))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(TxToken {
            device: &mut self.device,
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut capabilities = DeviceCapabilities::default();
        capabilities.max_transmission_unit = virtio_net::MTU;
        capabilities.medium = Medium::Ethernet;
        capabilities
    }
}
