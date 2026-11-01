use alloc::vec::Vec;
use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpCidr, Ipv4Address, Ipv4Cidr};

use crate::net_device::EthernetDevice;

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

    pub(crate) fn poll(&mut self, now_ns: u64) -> bool {
        let now_ms = (now_ns / 1_000_000).min(i64::MAX as u64) as i64;
        self.interface.poll(
            Instant::from_millis(now_ms),
            &mut self.device,
            &mut self.sockets,
        ) == smoltcp::iface::PollResult::SocketStateChanged
    }

    pub(crate) fn device_mut(&mut self) -> &mut EthernetDevice {
        &mut self.device
    }
}
