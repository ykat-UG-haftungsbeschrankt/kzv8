/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::kernel::error::Error;
use crate::kernel::event::r#loop::{EventFlags, EventLoop, EventSourceId, EventSourceToken};
use crate::kernel::net::arp::{ArpOperation, ArpPacket, ARP_PACKET_LENGTH};
use crate::kernel::net::device::EthernetDevice;
use crate::kernel::net::icmp::{
    write_destination_unreachable, write_echo_reply, write_time_exceeded,
};
use crate::kernel::net::tcp::connection::{TcpSocketTable, TcpTransmission};
use crate::kernel::net::tcp::segment::{write_segment, TcpSegment};
use crate::kernel::net::udp::{
    write_datagram, UdpDatagram, UdpSocketTable, UdpTransmission, MAX_UDP_PAYLOAD_LENGTH,
};
use crate::kernel::net::wire::{
    rewrite_forwarded_ipv4_header, write_ethernet_header, write_ipv4_header, EthernetFrame,
    Ipv4Address, MacAddress, Ipv4Packet,
    ETHERNET_HEADER_LENGTH, ETHER_TYPE_ARP, ETHER_TYPE_IPV4, IPV4_MINIMUM_HEADER_LENGTH,
    IPV4_PROTOCOL_ICMP, IPV4_PROTOCOL_TCP, IPV4_PROTOCOL_UDP,
};
use crate::kernel::vfs::plugin::tun::{self, TunDevice, TunTapMode};

pub(crate) const NETWORK_EVENT_SOURCE: EventSourceId = EventSourceId::new(1);
const MAX_ETHERNET_FRAME_LENGTH: usize = 1514;
const MAX_NEIGHBORS: usize = 16;
const MAX_PENDING_TRANSMISSIONS: usize = 16;
const MAX_PENDING_IPV4_TRANSMISSIONS: usize = 16;
const MAX_IPV4_ROUTES: usize = 16;
const RECEIVE_BUDGET: usize = 16;
const MAX_TUN_PEERS: usize = 16;
const NEIGHBOR_LIFETIME_NS: u64 = 60_000_000_000;
const DEFAULT_TTL: u8 = 64;
const LOCAL_ADDRESS: Ipv4Address = Ipv4Address([10, 0, 2, 15]);
const DEFAULT_GATEWAY: Ipv4Address = Ipv4Address([10, 0, 2, 2]);

#[derive(Clone, Copy)]
struct NeighborEntry {
    address: Ipv4Address,
    mac: MacAddress,
    expires_at_ns: u64,
}

struct TunnelPeer {
    address: Ipv4Address,
    device: Arc<TunDevice>,
    expires_at_ns: u64,
}

#[derive(Clone)]
struct Ipv4Route {
    network: Ipv4Address,
    prefix_length: u8,
    gateway: Option<Ipv4Address>,
    tunnel: Option<Arc<TunDevice>>,
}

struct Ipv4RouteTable {
    routes: Vec<Ipv4Route>,
}

impl Ipv4RouteTable {
    fn new() -> Option<Self> {
        let mut routes = Vec::new();
        routes.try_reserve_exact(MAX_IPV4_ROUTES).ok()?;
        routes.push(Ipv4Route {
            network: Ipv4Address([10, 0, 2, 0]),
            prefix_length: 24,
            gateway: None,
            tunnel: None,
        });
        routes.push(Ipv4Route {
            network: Ipv4Address([0; 4]),
            prefix_length: 0,
            gateway: Some(DEFAULT_GATEWAY),
            tunnel: None,
        });
        Some(Self { routes })
    }

    fn add(
        &mut self,
        network: Ipv4Address,
        prefix_length: u8,
        gateway: Option<Ipv4Address>,
    ) -> Result<(), Error> {
        if prefix_length > 32 {
            return Err(Error::Inval);
        }
        let network = mask_ipv4(network, prefix_length);
        if let Some(route) = self
            .routes
            .iter_mut()
            .find(|route| route.network == network && route.prefix_length == prefix_length)
        {
            route.gateway = gateway;
            route.tunnel = None;
            return Ok(());
        }
        if self.routes.len() == MAX_IPV4_ROUTES {
            return Err(Error::NoBufs);
        }
        self.routes.push(Ipv4Route {
            network,
            prefix_length,
            gateway,
            tunnel: None,
        });
        Ok(())
    }

    fn add_tunnel(
        &mut self,
        network: Ipv4Address,
        prefix_length: u8,
        device: Arc<TunDevice>,
    ) -> Result<(), Error> {
        if prefix_length > 32
            || device.mode() != TunTapMode::Tun
            || !device.is_configured()
            || !device.is_up()
        {
            return Err(Error::Inval);
        }
        let network = mask_ipv4(network, prefix_length);
        if let Some(route) = self
            .routes
            .iter_mut()
            .find(|route| route.network == network && route.prefix_length == prefix_length)
        {
            route.gateway = None;
            route.tunnel = Some(device);
            return Ok(());
        }
        if self.routes.len() == MAX_IPV4_ROUTES {
            return Err(Error::NoBufs);
        }
        self.routes.push(Ipv4Route {
            network,
            prefix_length,
            gateway: None,
            tunnel: Some(device),
        });
        Ok(())
    }

    fn remove(&mut self, network: Ipv4Address, prefix_length: u8) -> Result<(), Error> {
        if prefix_length > 32 {
            return Err(Error::Inval);
        }
        let network = mask_ipv4(network, prefix_length);
        let index = self
            .routes
            .iter()
            .position(|route| route.network == network && route.prefix_length == prefix_length)
            .ok_or(Error::NoEnt)?;
        self.routes.remove(index);
        Ok(())
    }

    fn next_hop(&self, destination: Ipv4Address) -> Option<Ipv4Address> {
        self.routes
            .iter()
            .filter(|route| mask_ipv4(destination, route.prefix_length) == route.network)
            .max_by_key(|route| route.prefix_length)
            .filter(|route| route.tunnel.is_none())
            .map(|route| route.gateway.unwrap_or(destination))
    }

    fn tunnel_device(&self, destination: Ipv4Address) -> Option<Arc<TunDevice>> {
        self.routes
            .iter()
            .filter(|route| mask_ipv4(destination, route.prefix_length) == route.network)
            .max_by_key(|route| route.prefix_length)
            .and_then(|route| {
                route
                    .tunnel
                    .as_ref()
                    .filter(|device| device.is_up())
                    .map(Arc::clone)
            })
    }

    fn is_broadcast_address(&self, address: Ipv4Address) -> bool {
        address == Ipv4Address([255; 4])
            || self.routes.iter().any(|route| {
                if route.gateway.is_some() || route.tunnel.is_some() || route.prefix_length == 32 {
                    return false;
                }
                let mask = mask_ipv4(Ipv4Address([255; 4]), route.prefix_length);
                route
                    .network
                    .0
                    .iter()
                    .zip(mask.0)
                    .zip(address.0)
                    .all(|((&network_octet, mask_octet), address_octet)| {
                        address_octet == (network_octet | !mask_octet)
                    })
            })
    }
}

struct PendingIpv4Transmission {
    next_hop: Ipv4Address,
    packet: Vec<u8>,
}

fn mask_ipv4(mut address: Ipv4Address, prefix_length: u8) -> Ipv4Address {
    let full_bytes = usize::from(prefix_length / 8);
    let remaining_bits = prefix_length % 8;
    let bytes_to_zero = full_bytes + if remaining_bits != 0 { 1 } else { 0 };
    for octet in address.0.iter_mut().skip(bytes_to_zero) {
        *octet = 0;
    }
    if remaining_bits != 0 {
        let mask = u8::MAX << (8 - remaining_bits);
        address.0[full_bytes] &= mask;
    }
    address
}

pub(crate) struct NetStack {
    pub(crate) tcp_sockets: TcpSocketTable,
    pub(crate) udp_sockets: UdpSocketTable,
    device: EthernetDevice,
    mac_address: MacAddress,
    local_address: Ipv4Address,
    neighbors: Vec<NeighborEntry>,
    tunnel_peers: Vec<TunnelPeer>,
    routes: Ipv4RouteTable,
    ipv4_forwarding_enabled: bool,
    pending_transmissions: VecDeque<TcpTransmission>,
    pending_udp_transmissions: VecDeque<UdpTransmission>,
    pending_ipv4_transmissions: VecDeque<PendingIpv4Transmission>,
    next_ipv4_identification: u16,
}

impl NetStack {
    pub(crate) fn initialize() -> Option<Self> {
        let mut device = EthernetDevice::initialize()?;
        let mac_address = MacAddress(device.mac_address());
        crate::kernel::vm::device::virtio::net::register_host_interface(mac_address.0);
        let mut neighbors = Vec::new();
        neighbors.try_reserve_exact(MAX_NEIGHBORS).ok()?;
        let mut tunnel_peers = Vec::new();
        tunnel_peers.try_reserve_exact(MAX_TUN_PEERS).ok()?;
        let mut pending_transmissions = VecDeque::new();
        pending_transmissions
            .try_reserve_exact(MAX_PENDING_TRANSMISSIONS)
            .ok()?;
        let mut pending_udp_transmissions = VecDeque::new();
        pending_udp_transmissions
            .try_reserve_exact(MAX_PENDING_TRANSMISSIONS)
            .ok()?;
        let mut pending_ipv4_transmissions = VecDeque::new();
        pending_ipv4_transmissions
            .try_reserve_exact(MAX_PENDING_IPV4_TRANSMISSIONS)
            .ok()?;
        Some(Self {
            tcp_sockets: TcpSocketTable::new()?,
            udp_sockets: UdpSocketTable::new()?,
            device,
            mac_address,
            local_address: LOCAL_ADDRESS,
            neighbors,
            tunnel_peers,
            routes: Ipv4RouteTable::new()?,
            ipv4_forwarding_enabled: false,
            pending_transmissions,
            pending_udp_transmissions,
            pending_ipv4_transmissions,
            next_ipv4_identification: 1,
        })
    }

    pub(crate) fn poll<const N: usize>(
        &mut self,
        now_ns: u64,
        events: &mut EventLoop<N>,
        source: EventSourceToken,
    ) -> bool {
        crate::kernel::vm::device::virtio::net::poll_tap_devices();
        let mut changed = false;
        let mut input = [0; MAX_ETHERNET_FRAME_LENGTH];
        changed |= self.poll_tun_devices(now_ns);
        for _ in 0..RECEIVE_BUDGET {
            let length = match self.device.poll_receive_frame() {
                Ok(Some(length)) if length <= input.len() => length,
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => break,
            };
            let Some(frame) = self.device.received_frame(length) else {
                continue;
            };
            input[..length].copy_from_slice(frame);
            crate::kernel::vm::device::virtio::net::host_ingress_frame(&input[..length]);
            changed |= self.process_frame(&input[..length], now_ns);
        }
        for _ in 0..RECEIVE_BUDGET {
            let Some(length) = crate::kernel::vm::device::virtio::net::receive_host_stack_frame(
                &mut input,
            ) else {
                break;
            };
            changed |= self.process_frame(&input[..length], now_ns);
        }
        self.expire_neighbors(now_ns);
        self.expire_tunnel_peers(now_ns);

        let mut transmissions = Vec::new();
        if self
            .tcp_sockets
            .poll(now_ns, &mut transmissions)
            .unwrap_or(false)
        {
            changed = true;
        }
        for transmission in transmissions {
            self.queue_tcp_transmission(transmission, now_ns);
        }
        self.flush_pending_transmissions(now_ns);
        self.flush_pending_udp_transmissions(now_ns);
        self.flush_pending_ipv4_transmissions(now_ns);
        self.flush_host_transmissions();
        if changed {
            let _ = events.set_readiness(source, EventFlags::READABLE);
        }
        changed
    }

    pub(crate) const fn ipv4_address(&self) -> Ipv4Address {
        self.local_address
    }

    pub(crate) const fn ethernet_mac_address(&self) -> [u8; 6] {
        self.mac_address.0
    }

    pub(crate) fn ethernet_mtu(&self) -> usize {
        self.device.mtu()
    }

    pub(crate) fn is_local_ipv4_address(&self, address: Ipv4Address) -> bool {
        address == self.local_address
            || tun::open_devices().is_ok_and(|devices| {
                devices.iter().any(|device| {
                    device.is_configured()
                        && device.is_up()
                        && device
                            .ipv4_configuration()
                            .is_some_and(|(interface_address, _)| interface_address == address)
                })
            })
    }

    pub(crate) fn set_ipv4_forwarding(&mut self, enabled: bool) {
        self.ipv4_forwarding_enabled = enabled;
    }

    pub(crate) fn add_ipv4_route(
        &mut self,
        network: Ipv4Address,
        prefix_length: u8,
        gateway: Option<Ipv4Address>,
    ) -> Result<(), Error> {
        self.routes.add(network, prefix_length, gateway)
    }

    pub(crate) fn add_ipv4_tun_route(
        &mut self,
        network: Ipv4Address,
        prefix_length: u8,
        device: Arc<TunDevice>,
    ) -> Result<(), Error> {
        self.routes.add_tunnel(network, prefix_length, device)
    }

    pub(crate) fn remove_ipv4_route(
        &mut self,
        network: Ipv4Address,
        prefix_length: u8,
    ) -> Result<(), Error> {
        self.routes.remove(network, prefix_length)
    }

    #[allow(dead_code)]
    pub(crate) fn device_mut(&mut self) -> &mut EthernetDevice {
        &mut self.device
    }

    #[allow(dead_code)]
    pub(crate) fn interrupt_slot(&self) -> usize {
        self.device.interrupt_slot()
    }

    fn poll_tun_devices(&mut self, now_ns: u64) -> bool {
        let Ok(devices) = tun::open_devices() else {
            return false;
        };
        let mut changed = false;
        for device in devices {
            if !device.is_configured() || device.mode() != TunTapMode::Tun {
                continue;
            }
            for _ in 0..RECEIVE_BUDGET {
                let Ok(packet) = device.dequeue_transmitted_packet() else {
                    break;
                };
                changed |= self.process_tun_packet(&device, &packet, now_ns);
            }
        }
        changed
    }

    fn process_tun_packet(&mut self, device: &Arc<TunDevice>, bytes: &[u8], now_ns: u64) -> bool {
        let Ok(packet) = Ipv4Packet::parse(bytes) else {
            return false;
        };
        let local_destination = self.is_local_ipv4_address(packet.destination)
            || (packet.protocol == IPV4_PROTOCOL_UDP
                && self.is_broadcast_address(packet.destination));
        if !local_destination {
            self.remember_tunnel_peer(packet.source, Arc::clone(device), now_ns);
            let ingress_address = device
                .ipv4_configuration()
                .map_or(self.local_address, |(address, _)| address);
            return self.forward_ipv4_packet(&packet, now_ns, ingress_address);
        }
        if packet.more_fragments || packet.fragment_offset != 0 {
            return false;
        }
        self.remember_tunnel_peer(packet.source, Arc::clone(device), now_ns);
        if packet.protocol == IPV4_PROTOCOL_UDP {
            let Ok(datagram) = UdpDatagram::parse(packet.payload, packet.source, packet.destination)
            else {
                return false;
            };
            return self
                .udp_sockets
                .receive_datagram(packet.source, packet.destination, &datagram)
                .unwrap_or(false);
        }
        if packet.protocol == IPV4_PROTOCOL_ICMP {
            let mut response = [0; MAX_ETHERNET_FRAME_LENGTH];
            let Ok(icmp_length) = write_echo_reply(packet.payload, &mut response[20..]) else {
                return false;
            };
            let Ok(ipv4_length) = write_ipv4_header(
                &mut response,
                self.local_address,
                packet.source,
                IPV4_PROTOCOL_ICMP,
                DEFAULT_TTL,
                self.next_ipv4_identification,
                0,
                icmp_length,
            ) else {
                return false;
            };
            self.next_ipv4_identification = self.next_ipv4_identification.wrapping_add(1);
            let _ = device.inject_packet(&response[..ipv4_length]);
            return true;
        }
        if packet.protocol != IPV4_PROTOCOL_TCP {
            return false;
        }
        let Ok(segment) = TcpSegment::parse(packet.payload, packet.source, packet.destination)
        else {
            return false;
        };
        let mut transmissions = Vec::new();
        let changed = self
            .tcp_sockets
            .receive_segment(
                packet.source,
                packet.destination,
                &segment,
                now_ns,
                &mut transmissions,
            )
            .unwrap_or(false);
        for transmission in transmissions {
            self.transmit_tcp_tun(transmission, device);
        }
        changed
    }

    fn process_frame(&mut self, bytes: &[u8], now_ns: u64) -> bool {
        let Ok(frame) = EthernetFrame::parse(bytes) else {
            return false;
        };
        if frame.destination != self.mac_address && !frame.destination.is_multicast() {
            return false;
        }
        match frame.ether_type {
            ETHER_TYPE_ARP => self.process_arp(frame.payload, now_ns),
            ETHER_TYPE_IPV4 => self.process_ipv4(frame, now_ns),
            _ => false,
        }
    }

    fn process_arp(&mut self, bytes: &[u8], now_ns: u64) -> bool {
        let Ok(packet) = ArpPacket::parse(bytes) else {
            return false;
        };
        self.remember_neighbor(packet.sender_ipv4, packet.sender_mac, now_ns);
        let is_local_address = self.is_local_ipv4_address(packet.target_ipv4);
        self.flush_pending_ipv4_transmissions(now_ns);
        let replied = packet.operation == ArpOperation::Request
            && is_local_address
            && self.send_arp_reply(packet.sender_mac, packet.sender_ipv4, packet.target_ipv4);
        self.flush_pending_transmissions(now_ns);
        replied
    }

    fn process_ipv4(&mut self, frame: EthernetFrame<'_>, now_ns: u64) -> bool {
        let Ok(packet) = Ipv4Packet::parse(frame.payload) else {
            return false;
        };
        let local_destination = self.is_local_ipv4_address(packet.destination)
            || (packet.protocol == IPV4_PROTOCOL_UDP
                && self.is_broadcast_address(packet.destination));
        if !local_destination {
            self.remember_neighbor(packet.source, frame.source, now_ns);
            return self.forward_ipv4_packet(&packet, now_ns, self.local_address);
        }
        if packet.more_fragments || packet.fragment_offset != 0 {
            return false;
        }
        self.remember_neighbor(packet.source, frame.source, now_ns);
        if packet.protocol == IPV4_PROTOCOL_UDP {
            let Ok(datagram) = UdpDatagram::parse(packet.payload, packet.source, packet.destination)
            else {
                return false;
            };
            return self
                .udp_sockets
                .receive_datagram(packet.source, packet.destination, &datagram)
                .unwrap_or(false);
        }
        if packet.protocol == IPV4_PROTOCOL_ICMP {
            let mut frame_bytes = [0; MAX_ETHERNET_FRAME_LENGTH];
            let icmp_offset = ETHERNET_HEADER_LENGTH + IPV4_MINIMUM_HEADER_LENGTH;
            let Ok(icmp_length) = write_echo_reply(
                packet.payload,
                &mut frame_bytes[icmp_offset..],
            ) else {
                return false;
            };
            let Ok(ipv4_length) = write_ipv4_header(
                &mut frame_bytes[ETHERNET_HEADER_LENGTH..],
                self.local_address,
                packet.source,
                IPV4_PROTOCOL_ICMP,
                DEFAULT_TTL,
                self.next_ipv4_identification,
                0,
                icmp_length,
            ) else {
                return false;
            };
            self.next_ipv4_identification = self.next_ipv4_identification.wrapping_add(1);
            if write_ethernet_header(
                &mut frame_bytes[..ETHERNET_HEADER_LENGTH],
                frame.source,
                self.mac_address,
                ETHER_TYPE_IPV4,
            )
            .is_err()
            {
                return false;
            }
            crate::kernel::vm::device::virtio::net::host_transmit_frame(
                &frame_bytes[..ETHERNET_HEADER_LENGTH + ipv4_length],
            );
            return true;
        }
        if packet.protocol != IPV4_PROTOCOL_TCP {
            return false;
        }
        let Ok(segment) = TcpSegment::parse(packet.payload, packet.source, packet.destination)
        else {
            return false;
        };
        let mut transmissions = Vec::new();
        let changed = self
            .tcp_sockets
            .receive_segment(
                packet.source,
                packet.destination,
                &segment,
                now_ns,
                &mut transmissions,
            )
            .unwrap_or(false);
        for transmission in transmissions {
            self.transmit_tcp(transmission, frame.source);
        }
        changed
    }

    fn forward_ipv4_packet(
        &mut self,
        packet: &Ipv4Packet<'_>,
        now_ns: u64,
        ingress_address: Ipv4Address,
    ) -> bool {
        if !self.ipv4_forwarding_enabled {
            return false;
        }
        if packet.ttl <= 1 {
            self.send_icmp_error(packet, ingress_address, now_ns, true, 0);
            return false;
        }
        let Some(packet_length) = packet.header.len().checked_add(packet.payload.len()) else {
            return false;
        };
        if packet_length > MAX_ETHERNET_FRAME_LENGTH - ETHERNET_HEADER_LENGTH {
            if packet.dont_fragment {
                self.send_icmp_error(packet, ingress_address, now_ns, false, 4);
            }
            return false;
        }
        let mut forwarded_packet = Vec::new();
        if forwarded_packet.try_reserve_exact(packet_length).is_err() {
            return false;
        }
        forwarded_packet.resize(packet_length, 0);
        let Ok(header_length) =
            rewrite_forwarded_ipv4_header(&mut forwarded_packet, packet.header)
        else {
            return false;
        };
        forwarded_packet[header_length..].copy_from_slice(packet.payload);
        if self.queue_ipv4_packet(packet.destination, forwarded_packet, now_ns) {
            return true;
        }
        if self.tunnel_peer(packet.destination, now_ns).is_none()
            && self.routes.tunnel_device(packet.destination).is_none()
            && self.next_hop(packet.destination).is_none()
        {
            self.send_icmp_error(packet, ingress_address, now_ns, false, 0);
        }
        false
    }

    fn send_icmp_error(
        &mut self,
        packet: &Ipv4Packet<'_>,
        source_address: Ipv4Address,
        now_ns: u64,
        time_exceeded: bool,
        unreachable_code: u8,
    ) {
        let mut icmp = [0; 8 + 60 + 8];
        let icmp_length = if time_exceeded {
            write_time_exceeded(packet.header, packet.payload, &mut icmp)
        } else {
            write_destination_unreachable(
                unreachable_code,
                packet.header,
                packet.payload,
                &mut icmp,
            )
        };
        let Ok(icmp_length) = icmp_length else {
            return;
        };
        let mut ip_packet = [0; 20 + 8 + 60 + 8];
        let Ok(ip_length) = write_ipv4_header(
            &mut ip_packet,
            source_address,
            packet.source,
            IPV4_PROTOCOL_ICMP,
            DEFAULT_TTL,
            self.next_ipv4_identification,
            0,
            icmp_length,
        ) else {
            return;
        };
        self.next_ipv4_identification = self.next_ipv4_identification.wrapping_add(1);
        ip_packet[20..ip_length].copy_from_slice(&icmp[..icmp_length]);
        let mut response = Vec::new();
        if response.try_reserve_exact(ip_length).is_err() {
            return;
        }
        response.extend_from_slice(&ip_packet[..ip_length]);
        let _queued = self.queue_ipv4_packet(packet.source, response, now_ns);
    }

    fn queue_ipv4_packet(
        &mut self,
        destination: Ipv4Address,
        packet: Vec<u8>,
        now_ns: u64,
    ) -> bool {
        if let Some(device) = self.tunnel_peer(destination, now_ns) {
            return device.inject_packet(&packet).is_ok();
        }
        if let Some(device) = self.routes.tunnel_device(destination) {
            return device.inject_packet(&packet).is_ok();
        }
        let Some(next_hop) = self.next_hop(destination) else {
            return false;
        };
        if let Some(mac_address) = self.neighbor_mac(next_hop, now_ns) {
            self.transmit_forwarded_ipv4(&packet, mac_address);
            return true;
        }
        if self.pending_ipv4_transmissions.len() == MAX_PENDING_IPV4_TRANSMISSIONS {
            return false;
        }
        let request_pending = self
            .pending_ipv4_transmissions
            .iter()
            .any(|pending| pending.next_hop == next_hop);
        self.pending_ipv4_transmissions
            .push_back(PendingIpv4Transmission {
                next_hop,
                packet,
            });
        if !request_pending {
            self.send_arp_request(next_hop);
        }
        true
    }

    fn transmit_forwarded_ipv4(&mut self, packet: &[u8], destination_mac: MacAddress) {
        if packet.len() > MAX_ETHERNET_FRAME_LENGTH - ETHERNET_HEADER_LENGTH {
            return;
        }
        let mut frame = [0; MAX_ETHERNET_FRAME_LENGTH];
        if write_ethernet_header(
            &mut frame[..ETHERNET_HEADER_LENGTH],
            destination_mac,
            self.mac_address,
            ETHER_TYPE_IPV4,
        )
        .is_err()
        {
            return;
        }
        frame[ETHERNET_HEADER_LENGTH..ETHERNET_HEADER_LENGTH + packet.len()]
            .copy_from_slice(packet);
        crate::kernel::vm::device::virtio::net::host_transmit_frame(
            &frame[..ETHERNET_HEADER_LENGTH + packet.len()],
        );
    }

    fn queue_tcp_transmission(&mut self, transmission: TcpTransmission, now_ns: u64) {
        if let Some(device) = self.tunnel_peer(transmission.destination_address, now_ns) {
            self.transmit_tcp_tun(transmission, &device);
            return;
        }
        if let Some(device) = self.routes.tunnel_device(transmission.destination_address) {
            self.transmit_tcp_tun(transmission, &device);
            return;
        }
        let Some(next_hop) = self.next_hop(transmission.destination_address) else {
            return;
        };
        if let Some(mac_address) = self.neighbor_mac(next_hop, now_ns) {
            self.transmit_tcp(transmission, mac_address);
            return;
        }
        let request_pending = self
            .pending_transmissions
            .iter()
            .any(|pending| self.next_hop(pending.destination_address) == Some(next_hop));
        if self.pending_transmissions.len() < MAX_PENDING_TRANSMISSIONS {
            self.pending_transmissions.push_back(transmission);
        }
        if !request_pending {
            self.send_arp_request(next_hop);
        }
    }

    fn flush_pending_transmissions(&mut self, now_ns: u64) {
        let pending_count = self.pending_transmissions.len();
        for _ in 0..pending_count {
            let Some(transmission) = self.pending_transmissions.pop_front() else {
                break;
            };
            let Some(next_hop) = self.next_hop(transmission.destination_address) else {
                continue;
            };
            if let Some(mac_address) = self.neighbor_mac(next_hop, now_ns) {
                self.transmit_tcp(transmission, mac_address);
            } else {
                self.pending_transmissions.push_back(transmission);
            }
        }
    }

    pub(crate) fn send_udp(
        &mut self,
        handle: crate::kernel::net::udp::UdpSocketHandle,
        destination: Ipv4Address,
        destination_port: u16,
        payload: &[u8],
        now_ns: u64,
    ) -> Result<usize, Error> {
        if payload.len() > MAX_UDP_PAYLOAD_LENGTH {
            return Err(Error::MsgSize);
        }
        let transmission = self.udp_sockets.send_to(
            handle,
            self.local_address,
            destination,
            destination_port,
            payload,
        )?;
        let length = transmission.payload.len();
        self.queue_udp_transmission(transmission, now_ns);
        Ok(length)
    }

    fn queue_udp_transmission(&mut self, transmission: UdpTransmission, now_ns: u64) {
        if let Some(device) = self.routes.tunnel_device(transmission.destination_address) {
            self.transmit_udp_tun(transmission, &device);
            return;
        }
        if self.is_broadcast_address(transmission.destination_address) {
            self.transmit_udp(transmission, MacAddress::BROADCAST);
            return;
        }
        if let Some(device) = self.tunnel_peer(transmission.destination_address, now_ns) {
            self.transmit_udp_tun(transmission, &device);
            return;
        }
        let Some(next_hop) = self.next_hop(transmission.destination_address) else {
            return;
        };
        if let Some(mac_address) = self.neighbor_mac(next_hop, now_ns) {
            self.transmit_udp(transmission, mac_address);
            return;
        }
        let request_pending = self
            .pending_udp_transmissions
            .iter()
            .any(|pending| self.next_hop(pending.destination_address) == Some(next_hop));
        if self.pending_udp_transmissions.len() < MAX_PENDING_TRANSMISSIONS {
            self.pending_udp_transmissions.push_back(transmission);
        }
        if !request_pending {
            self.send_arp_request(next_hop);
        }
    }

    fn flush_pending_udp_transmissions(&mut self, now_ns: u64) {
        let pending_count = self.pending_udp_transmissions.len();
        for _ in 0..pending_count {
            let Some(transmission) = self.pending_udp_transmissions.pop_front() else {
                break;
            };
            let Some(next_hop) = self.next_hop(transmission.destination_address) else {
                continue;
            };
            if let Some(mac_address) = self.neighbor_mac(next_hop, now_ns) {
                self.transmit_udp(transmission, mac_address);
            } else {
                self.pending_udp_transmissions.push_back(transmission);
            }
        }
    }

    fn flush_pending_ipv4_transmissions(&mut self, now_ns: u64) {
        let pending_count = self.pending_ipv4_transmissions.len();
        for _ in 0..pending_count {
            let Some(pending) = self.pending_ipv4_transmissions.pop_front() else {
                break;
            };
            if let Some(mac_address) = self.neighbor_mac(pending.next_hop, now_ns) {
                self.transmit_forwarded_ipv4(&pending.packet, mac_address);
            } else {
                self.pending_ipv4_transmissions.push_back(pending);
            }
        }
    }

    fn transmit_udp(&mut self, transmission: UdpTransmission, destination_mac: MacAddress) {
        let mut frame = [0; MAX_ETHERNET_FRAME_LENGTH];
        let udp_offset = ETHERNET_HEADER_LENGTH + IPV4_MINIMUM_HEADER_LENGTH;
        let Ok(udp_length) = write_datagram(
            &mut frame[udp_offset..],
            transmission.source_address,
            transmission.destination_address,
            transmission.source_port,
            transmission.destination_port,
            &transmission.payload,
        ) else {
            return;
        };
        let Ok(ipv4_length) = write_ipv4_header(
            &mut frame[ETHERNET_HEADER_LENGTH..],
            transmission.source_address,
            transmission.destination_address,
            IPV4_PROTOCOL_UDP,
            DEFAULT_TTL,
            self.next_ipv4_identification,
            0,
            udp_length,
        ) else {
            return;
        };
        self.next_ipv4_identification = self.next_ipv4_identification.wrapping_add(1);
        if write_ethernet_header(
            &mut frame[..ETHERNET_HEADER_LENGTH],
            destination_mac,
            self.mac_address,
            ETHER_TYPE_IPV4,
        )
        .is_err()
        {
            return;
        }
        crate::kernel::vm::device::virtio::net::host_transmit_frame(
            &frame[..ETHERNET_HEADER_LENGTH + ipv4_length],
        );
    }

    fn transmit_tcp_tun(&mut self, transmission: TcpTransmission, device: &TunDevice) {
        let mut packet = [0; MAX_ETHERNET_FRAME_LENGTH];
        let tcp_offset = IPV4_MINIMUM_HEADER_LENGTH;
        let Ok(tcp_length) = write_segment(
            &mut packet[tcp_offset..],
            transmission.source_address,
            transmission.destination_address,
            transmission.source_port,
            transmission.destination_port,
            transmission.sequence_number,
            transmission.acknowledgment_number,
            transmission.flags,
            transmission.window,
            &transmission.payload,
        ) else {
            return;
        };
        let Ok(ipv4_length) = write_ipv4_header(
            &mut packet,
            transmission.source_address,
            transmission.destination_address,
            IPV4_PROTOCOL_TCP,
            DEFAULT_TTL,
            self.next_ipv4_identification,
            0,
            tcp_length,
        ) else {
            return;
        };
        self.next_ipv4_identification = self.next_ipv4_identification.wrapping_add(1);
        let _ = device.inject_packet(&packet[..ipv4_length]);
    }

    fn transmit_udp_tun(&mut self, transmission: UdpTransmission, device: &TunDevice) {
        let mut packet = [0; MAX_ETHERNET_FRAME_LENGTH];
        let udp_offset = IPV4_MINIMUM_HEADER_LENGTH;
        let Ok(udp_length) = write_datagram(
            &mut packet[udp_offset..],
            transmission.source_address,
            transmission.destination_address,
            transmission.source_port,
            transmission.destination_port,
            &transmission.payload,
        ) else {
            return;
        };
        let Ok(ipv4_length) = write_ipv4_header(
            &mut packet,
            transmission.source_address,
            transmission.destination_address,
            IPV4_PROTOCOL_UDP,
            DEFAULT_TTL,
            self.next_ipv4_identification,
            0,
            udp_length,
        ) else {
            return;
        };
        self.next_ipv4_identification = self.next_ipv4_identification.wrapping_add(1);
        let _ = device.inject_packet(&packet[..ipv4_length]);
    }

    fn transmit_tcp(&mut self, transmission: TcpTransmission, destination_mac: MacAddress) {
        let mut frame = [0; MAX_ETHERNET_FRAME_LENGTH];
        let tcp_offset = ETHERNET_HEADER_LENGTH + IPV4_MINIMUM_HEADER_LENGTH;
        let Ok(tcp_length) = write_segment(
            &mut frame[tcp_offset..],
            transmission.source_address,
            transmission.destination_address,
            transmission.source_port,
            transmission.destination_port,
            transmission.sequence_number,
            transmission.acknowledgment_number,
            transmission.flags,
            transmission.window,
            &transmission.payload,
        ) else {
            return;
        };
        let Ok(ipv4_length) = write_ipv4_header(
            &mut frame[ETHERNET_HEADER_LENGTH..],
            transmission.source_address,
            transmission.destination_address,
            IPV4_PROTOCOL_TCP,
            DEFAULT_TTL,
            self.next_ipv4_identification,
            0,
            tcp_length,
        ) else {
            return;
        };
        self.next_ipv4_identification = self.next_ipv4_identification.wrapping_add(1);
        if write_ethernet_header(
            &mut frame[..ETHERNET_HEADER_LENGTH],
            destination_mac,
            self.mac_address,
            ETHER_TYPE_IPV4,
        )
        .is_err()
        {
            return;
        }
        let frame_length = ETHERNET_HEADER_LENGTH + ipv4_length;
        crate::kernel::vm::device::virtio::net::host_transmit_frame(&frame[..frame_length]);
    }

    fn send_arp_request(&mut self, target_address: Ipv4Address) {
        let packet = ArpPacket {
            operation: ArpOperation::Request,
            sender_mac: self.mac_address,
            sender_ipv4: self.local_address,
            target_mac: MacAddress([0; 6]),
            target_ipv4: target_address,
        };
        self.transmit_arp(MacAddress::BROADCAST, packet);
    }

    fn send_arp_reply(
        &mut self,
        destination_mac: MacAddress,
        destination_address: Ipv4Address,
        sender_address: Ipv4Address,
    ) -> bool {
        let packet = ArpPacket {
            operation: ArpOperation::Reply,
            sender_mac: self.mac_address,
            sender_ipv4: sender_address,
            target_mac: destination_mac,
            target_ipv4: destination_address,
        };
        self.transmit_arp(destination_mac, packet)
    }

    fn transmit_arp(&mut self, destination_mac: MacAddress, packet: ArpPacket) -> bool {
        let mut frame = [0; ETHERNET_HEADER_LENGTH + ARP_PACKET_LENGTH];
        if write_ethernet_header(
            &mut frame[..ETHERNET_HEADER_LENGTH],
            destination_mac,
            self.mac_address,
            ETHER_TYPE_ARP,
        )
        .is_err()
            || packet.write(&mut frame[ETHERNET_HEADER_LENGTH..]).is_err()
        {
            return false;
        }
        crate::kernel::vm::device::virtio::net::host_transmit_frame(&frame);
        true
    }

    fn remember_neighbor(&mut self, address: Ipv4Address, mac: MacAddress, now_ns: u64) {
        if address == Ipv4Address([0; 4]) || mac.is_multicast() {
            return;
        }
        let expires_at_ns = now_ns.saturating_add(NEIGHBOR_LIFETIME_NS);
        if let Some(entry) = self
            .neighbors
            .iter_mut()
            .find(|entry| entry.address == address)
        {
            entry.mac = mac;
            entry.expires_at_ns = expires_at_ns;
            return;
        }
        if self.neighbors.len() == MAX_NEIGHBORS {
            if let Some((index, _)) = self
                .neighbors
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.expires_at_ns)
            {
                self.neighbors[index] = NeighborEntry {
                    address,
                    mac,
                    expires_at_ns,
                };
            }
        } else {
            self.neighbors.push(NeighborEntry {
                address,
                mac,
                expires_at_ns,
            });
        }
    }

    fn neighbor_mac(&self, address: Ipv4Address, now_ns: u64) -> Option<MacAddress> {
        self.neighbors
            .iter()
            .find(|entry| entry.address == address && entry.expires_at_ns > now_ns)
            .map(|entry| entry.mac)
    }

    fn expire_neighbors(&mut self, now_ns: u64) {
        self.neighbors
            .retain(|entry| entry.expires_at_ns > now_ns);
    }

    fn remember_tunnel_peer(
        &mut self,
        address: Ipv4Address,
        device: Arc<TunDevice>,
        now_ns: u64,
    ) {
        let expires_at_ns = now_ns.saturating_add(NEIGHBOR_LIFETIME_NS);
        if let Some(peer) = self
            .tunnel_peers
            .iter_mut()
            .find(|peer| peer.address == address)
        {
            peer.device = device;
            peer.expires_at_ns = expires_at_ns;
            return;
        }
        if self.tunnel_peers.len() == MAX_TUN_PEERS {
            if let Some((index, _)) = self
                .tunnel_peers
                .iter()
                .enumerate()
                .min_by_key(|(_, peer)| peer.expires_at_ns)
            {
                self.tunnel_peers[index] = TunnelPeer {
                    address,
                    device,
                    expires_at_ns,
                };
            }
        } else {
            self.tunnel_peers.push(TunnelPeer {
                address,
                device,
                expires_at_ns,
            });
        }
    }

    fn tunnel_peer(&self, address: Ipv4Address, now_ns: u64) -> Option<Arc<TunDevice>> {
        self.tunnel_peers
            .iter()
            .find(|peer| peer.address == address && peer.expires_at_ns > now_ns)
            .map(|peer| Arc::clone(&peer.device))
    }

    fn expire_tunnel_peers(&mut self, now_ns: u64) {
        self.tunnel_peers
            .retain(|peer| peer.expires_at_ns > now_ns);
    }

    fn flush_host_transmissions(&mut self) {
        let mut frame = [0; MAX_ETHERNET_FRAME_LENGTH];
        for _ in 0..RECEIVE_BUDGET {
            let Some(length) =
                crate::kernel::vm::device::virtio::net::receive_host_transmit_frame(&mut frame)
            else {
                break;
            };
            let _ = self.device.transmit_frame(&frame[..length]);
        }
    }

    fn next_hop(&self, destination: Ipv4Address) -> Option<Ipv4Address> {
        self.routes.next_hop(destination)
    }

    fn is_broadcast_address(&self, address: Ipv4Address) -> bool {
        self.routes.is_broadcast_address(address)
    }
}

#[cfg(test)]
mod tests {
    use super::{Ipv4Address, Ipv4RouteTable, DEFAULT_GATEWAY};

    #[test]
    fn route_lookup_prefers_the_longest_prefix() {
        let mut routes = Ipv4RouteTable::new().unwrap();
        routes
            .add(
                Ipv4Address([10, 0, 2, 0]),
                24,
                Some(Ipv4Address([10, 0, 2, 1])),
            )
            .unwrap();
        routes
            .add(
                Ipv4Address([10, 0, 2, 128]),
                25,
                Some(Ipv4Address([10, 0, 2, 2])),
            )
            .unwrap();
        assert_eq!(
            routes.next_hop(Ipv4Address([10, 0, 2, 200])),
            Some(Ipv4Address([10, 0, 2, 2]))
        );
        assert_eq!(
            routes.next_hop(Ipv4Address([203, 0, 113, 1])),
            Some(DEFAULT_GATEWAY)
        );
    }

    #[test]
    fn deleted_default_route_stops_fallback_routing() {
        let mut routes = Ipv4RouteTable::new().unwrap();
        routes.remove(Ipv4Address([0; 4]), 0).unwrap();
        assert_eq!(routes.next_hop(Ipv4Address([203, 0, 113, 1])), None);
    }
}
