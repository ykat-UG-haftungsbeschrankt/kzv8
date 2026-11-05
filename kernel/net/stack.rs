/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpCidr, Ipv4Address, Ipv4Cidr};

use crate::kernel::event::r#loop::{EventFlags, EventLoop, EventSourceId, EventSourceToken};
use crate::kernel::net::device::EthernetDevice;

pub(crate) const NETWORK_EVENT_SOURCE: EventSourceId = EventSourceId::new(1);

pub(crate) struct NetStack {
    pub(crate) interface: Interface,
    pub(crate) sockets: SocketSet<'static>,
    device: EthernetDevice,
}

impl NetStack {
    pub(crate) fn initialize() -> Option<Self> {
        let mut device = EthernetDevice::initialize()?;
        let mac = device.mac_address();
        let config = Config::new(HardwareAddress::Ethernet(EthernetAddress(mac)));
        let now = Instant::from_millis(0);
        let mut interface = Interface::new(config, &mut device, now);
        interface.update_ip_addrs(|addresses| {
            addresses
                .push(IpCidr::Ipv4(Ipv4Cidr::new(
                    Ipv4Address::new(10, 0, 2, 15),
                    24,
                )))
                .expect("interface address capacity exhausted");
        });
        interface
            .routes_mut()
            .add_default_ipv4_route(Ipv4Address::new(10, 0, 2, 2))
            .expect("route table capacity exhausted");
        let sockets = SocketSet::new(Vec::<SocketStorage<'static>>::new());
        Some(Self {
            interface,
            sockets,
            device,
        })
    }

    pub(crate) fn poll<const N: usize>(
        &mut self,
        now_ns: u64,
        events: &mut EventLoop<N>,
        source: EventSourceToken,
    ) -> bool {
        let now_ms = (now_ns / 1_000_000).min(i64::MAX as u64) as i64;
        let changed = self.interface.poll(
            Instant::from_millis(now_ms),
            &mut self.device,
            &mut self.sockets,
        ) == smoltcp::iface::PollResult::SocketStateChanged;
        if changed {
            let _ = events.set_readiness(source, EventFlags::READABLE);
        }
        changed
    }

    #[allow(dead_code)]
    pub(crate) fn device_mut(&mut self) -> &mut EthernetDevice {
        &mut self.device
    }

    #[allow(dead_code)]
    pub(crate) fn interrupt_slot(&self) -> usize {
        self.device.interrupt_slot()
    }
}
