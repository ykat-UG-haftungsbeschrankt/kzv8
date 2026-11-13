/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
            let Some(index) = self
                .tunnel_peers
                .iter()
                .enumerate()
                .min_by_key(|(_, peer)| peer.expires_at_ns)
                .map(|(index, _)| index)
            else {
                return false;
            };
            self.tunnel_peers[index] = TunnelPeer {
                address,
                device,
                expires_at_ns,
            };
            return true;
 */

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::kernel::error::Error;
use crate::kernel::event::{
    EventFlags, EventFlagsApi, EventLoop, EventLoopApi as _, EventSourceId, EventSourceToken,
};
use crate::kernel::net::arp::{ArpOperation, ArpPacket, ARP_PACKET_LENGTH};
use crate::kernel::net::device::EthernetDevice;
use crate::kernel::net::icmp::{
    write_destination_unreachable, write_echo_reply, write_fragmentation_needed,
    write_time_exceeded,
};
use crate::kernel::net::ipv4_fragments::Ipv4FragmentReassembler;
use crate::kernel::net::tcp::connection::{TcpSocketTable, TcpTransmission};
use crate::kernel::net::tcp::segment::{write_segment, TcpSegment};
use crate::kernel::net::udp::{
    write_datagram, UdpDatagram, UdpSocketTable, UdpTransmission, UDP_HEADER_LENGTH,
};
use crate::kernel::net::wire::{
    fragment_ipv4_packet, rewrite_forwarded_ipv4_header, write_ethernet_header, write_ipv4_header, EthernetFrame,
    Ipv4Address, MacAddress, Ipv4Packet,
    ETHERNET_HEADER_LENGTH, ETHER_TYPE_ARP, ETHER_TYPE_IPV4, IPV4_MINIMUM_HEADER_LENGTH,
    IPV4_PROTOCOL_ICMP, IPV4_PROTOCOL_TCP, IPV4_PROTOCOL_UDP,
};
use crate::kernel::vfs::plugin::tun::{self, TunDevice, TunTapMode};

// Supported networking is Ethernet/ARP with IPv4, TCP, UDP, and ICMP echo/errors.
// Local IPv4 fragments are reassembled; forwarded fragments remain fragmented.
// IPv6, DHCP/DNS services, NAT, and raw sockets are outside this contract.
pub(crate) const NETWORK_EVENT_SOURCE: EventSourceId = EventSourceId::new(1);
const INITIAL_ROUTE_CAPACITY: usize = 2;
const INITIAL_RECEIVE_BUDGET: usize = 16;
const INITIAL_PENDING_CAPACITY: usize = 16;
const MAX_PENDING_ROUTE_ENTRIES: usize = 256;
const MAX_PENDING_ROUTE_BYTES: usize = 1024 * 1024;
const MAX_NEIGHBORS: usize = 256;
const MAX_TUNNEL_PEERS: usize = 256;
const TUN_INTERFACE_MTU: usize = 1500;
const ARP_RETRY_INTERVAL_NS: u64 = 1_000_000_000;
const MAX_ARP_RETRY_INTERVAL_NS: u64 = 8_000_000_000;
const MAX_ARP_REQUEST_ATTEMPTS: u8 = 5;
const NEIGHBOR_LIFETIME_NS: u64 = 60_000_000_000;
const INITIAL_TTL: u8 = 64;
const INITIAL_LOCAL_ADDRESS: Ipv4Address = Ipv4Address([10, 0, 2, 15]);
const INITIAL_PREFIX_LENGTH: u8 = 24;
const INITIAL_DEFAULT_GATEWAY: Ipv4Address = Ipv4Address([10, 0, 2, 2]);

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

struct PendingArpRequest {
    address: Ipv4Address,
    next_attempt_ns: u64,
    attempts: u8,
}

#[derive(Clone)]
struct Ipv4Route {
    network: Ipv4Address,
    prefix_length: u8,
    gateway: Option<Ipv4Address>,
    tunnel: Option<Arc<TunDevice>>,
}

struct Ipv4RouteTable {
    by_prefix: Vec<Vec<Ipv4Route>>,
}

impl Ipv4RouteTable {
    fn new(
        local_address: Ipv4Address,
        prefix_length: u8,
        default_gateway: Ipv4Address,
    ) -> Option<Self> {
        let mut by_prefix = Vec::new();
        by_prefix.try_reserve_exact(33).ok()?;
        for _ in 0..=32 {
            let mut routes = Vec::new();
            routes.try_reserve_exact(INITIAL_ROUTE_CAPACITY).ok()?;
            by_prefix.push(routes);
        }
        let mut table = Self { by_prefix };
        table.add(local_address, prefix_length, None).ok()?;
        table.add(Ipv4Address([0; 4]), 0, Some(default_gateway)).ok()?;
        Some(table)
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
        self.upsert(Ipv4Route {
            network: mask_ipv4(network, prefix_length),
            prefix_length,
            gateway,
            tunnel: None,
        })
    }

    fn upsert(&mut self, route: Ipv4Route) -> Result<(), Error> {
        let routes = self
            .by_prefix
            .get_mut(usize::from(route.prefix_length))
            .ok_or(Error::Inval)?;
        match routes.binary_search_by_key(&route.network, |entry| entry.network) {
            Ok(index) => routes[index] = route,
            Err(index) => {
                routes.try_reserve(1).map_err(|_| Error::NoMem)?;
                routes.insert(index, route);
            }
        }
        Ok(())
    }

    fn remove_exact(
        &mut self,
        network: Ipv4Address,
        prefix_length: u8,
    ) -> Option<Ipv4Route> {
        let routes = self.by_prefix.get_mut(usize::from(prefix_length))?;
        let index = routes.binary_search_by_key(&network, |route| route.network).ok()?;
        Some(routes.remove(index))
    }

    fn update_connected_route(
        &mut self,
        old_address: Ipv4Address,
        old_prefix_length: u8,
        new_address: Ipv4Address,
        new_prefix_length: u8,
    ) -> Result<(), Error> {
        let old_network = mask_ipv4(old_address, old_prefix_length);
        let new_network = mask_ipv4(new_address, new_prefix_length);
        if old_network == new_network && old_prefix_length == new_prefix_length {
            return Ok(());
        }
        let old_connected_route = self
            .by_prefix
            .get(usize::from(old_prefix_length))
            .and_then(|routes| routes.binary_search_by_key(&old_network, |route| route.network).ok().map(|index| &routes[index]))
            .is_some_and(|route| route.gateway.is_none() && route.tunnel.is_none());
        self.add(new_address, new_prefix_length, None)?;
        if old_connected_route {
            let _removed = self.remove_exact(old_network, old_prefix_length);
        }
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
        self.upsert(Ipv4Route {
            network,
            prefix_length,
            gateway: None,
            tunnel: Some(device),
        })
    }

    fn remove(&mut self, network: Ipv4Address, prefix_length: u8) -> Result<(), Error> {
        if prefix_length > 32 {
            return Err(Error::Inval);
        }
        let network = mask_ipv4(network, prefix_length);
        self.remove_exact(network, prefix_length)
            .map(|_| ())
            .ok_or(Error::NoEnt)
    }

    fn next_hop(&self, destination: Ipv4Address) -> Option<Ipv4Address> {
        for prefix_length in (0..=32).rev() {
            let network = mask_ipv4(destination, prefix_length);
            let routes = &self.by_prefix[usize::from(prefix_length)];
            if let Ok(index) = routes.binary_search_by_key(&network, |route| route.network) {
                let route = &routes[index];
                return route.tunnel.is_none().then_some(route.gateway.unwrap_or(destination));
            }
        }
        None
    }

    fn tunnel_device(&self, destination: Ipv4Address) -> Option<Arc<TunDevice>> {
        for prefix_length in (0..=32).rev() {
            let network = mask_ipv4(destination, prefix_length);
            let routes = &self.by_prefix[usize::from(prefix_length)];
            if let Ok(index) = routes.binary_search_by_key(&network, |route| route.network) {
                return routes[index]
                    .tunnel
                    .as_ref()
                    .filter(|device| device.is_up())
                    .map(Arc::clone);
            }
        }
        None
    }

    fn uses_tunnel(&self, destination: Ipv4Address) -> bool {
        for prefix_length in (0..=32).rev() {
            let network = mask_ipv4(destination, prefix_length);
            let routes = &self.by_prefix[usize::from(prefix_length)];
            if let Ok(index) = routes.binary_search_by_key(&network, |route| route.network) {
                return routes[index].tunnel.is_some();
            }
        }
        false
    }

    fn is_broadcast_address(&self, address: Ipv4Address) -> bool {
        if address == Ipv4Address([255; 4]) {
            return true;
        }
        for prefix_length in (0..=32).rev() {
            let network = mask_ipv4(address, prefix_length);
            let routes = &self.by_prefix[usize::from(prefix_length)];
            let Ok(index) = routes.binary_search_by_key(&network, |route| route.network) else {
                continue;
            };
            let route = &routes[index];
            if prefix_length >= 31 || route.gateway.is_some() {
                return false;
            }
            let mask = mask_ipv4(Ipv4Address([255; 4]), prefix_length);
            let broadcast = Ipv4Address(core::array::from_fn(|octet| {
                route.network.0[octet] | !mask.0[octet]
            }));
            if broadcast == address {
                return true;
            }
        }
        false
    }
}

struct PendingIpv4Transmission {
    next_hop: Ipv4Address,
    destination: Ipv4Address,
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

fn allocate_zeroed(length: usize) -> Option<Vec<u8>> {
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(length).ok()?;
    buffer.resize(length, 0);
    Some(buffer)
}

pub(crate) struct NetStack {
    pub(crate) tcp_sockets: TcpSocketTable,
    pub(crate) udp_sockets: UdpSocketTable,
    device: EthernetDevice,
    frame_size: usize,
    mac_address: MacAddress,
    local_address: Ipv4Address,
    local_prefix_length: u8,
    default_ttl: u8,
    receive_budget: usize,
    fragment_reassembler: Ipv4FragmentReassembler,
    neighbors: Vec<NeighborEntry>,
    tunnel_peers: Vec<TunnelPeer>,
    arp_requests: Vec<PendingArpRequest>,
    routes: Ipv4RouteTable,
    ipv4_forwarding_enabled: bool,
    pending_ipv4_transmissions: VecDeque<PendingIpv4Transmission>,
    pending_route_bytes: usize,
    next_ipv4_identification: u16,
}

impl NetStack {
    pub(crate) fn initialize() -> Option<Self> {
        let mut device = EthernetDevice::initialize()?;
        let mac_address = MacAddress(device.mac_address());
        let frame_size = device.max_frame_size();
        let local_address = INITIAL_LOCAL_ADDRESS;
        let local_prefix_length = INITIAL_PREFIX_LENGTH;
        #[cfg(feature = "virtualization")]
        crate::kernel::vm::device::virtio::net::register_host_interface(mac_address.0);
        let mut neighbors = Vec::new();
        neighbors.try_reserve_exact(INITIAL_PENDING_CAPACITY).ok()?;
        let mut tunnel_peers = Vec::new();
        tunnel_peers.try_reserve_exact(INITIAL_PENDING_CAPACITY).ok()?;
        let mut arp_requests = Vec::new();
        arp_requests.try_reserve_exact(MAX_PENDING_ROUTE_ENTRIES).ok()?;
        let mut pending_ipv4_transmissions = VecDeque::new();
        pending_ipv4_transmissions
            .try_reserve_exact(INITIAL_PENDING_CAPACITY)
            .ok()?;
        Some(Self {
            tcp_sockets: TcpSocketTable::new(
                device.max_frame_size().saturating_sub(ETHERNET_HEADER_LENGTH),
            )?,
            udp_sockets: UdpSocketTable::new()?,
            device,
            frame_size,
            mac_address,
            local_address,
            local_prefix_length,
            default_ttl: INITIAL_TTL,
            receive_budget: INITIAL_RECEIVE_BUDGET,
            fragment_reassembler: Ipv4FragmentReassembler::new(),
            neighbors,
            tunnel_peers,
            arp_requests,
            routes: Ipv4RouteTable::new(
                local_address,
                local_prefix_length,
                INITIAL_DEFAULT_GATEWAY,
            )?,
            ipv4_forwarding_enabled: false,
            pending_ipv4_transmissions,
            pending_route_bytes: 0,
            next_ipv4_identification: 1,
        })
    }

    pub(crate) fn poll<const N: usize>(
        &mut self,
        now_ns: u64,
        events: &mut EventLoop<N>,
        source: EventSourceToken,
    ) -> bool {
        #[cfg(feature = "virtualization")]
        crate::kernel::vm::device::virtio::net::poll_tap_devices();
        let mut changed = false;
        let Some(mut input) = allocate_zeroed(self.frame_size) else {
            return false;
        };
        changed |= self.poll_tun_devices(now_ns, self.receive_budget);
        for _ in 0..self.receive_budget {
            let length = match self.device.poll_receive_frame() {
                Ok(Some(length)) if length <= input.len() => length,
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => break,
            };
            let Some(frame) = self.device.received_frame(length) else {
                continue;
            };
            input[..length].copy_from_slice(frame);
            #[cfg(feature = "virtualization")]
            crate::kernel::vm::device::virtio::net::host_ingress_frame(&input[..length]);
            changed |= self.process_frame(&input[..length], now_ns);
        }
        #[cfg(feature = "virtualization")]
        for _ in 0..self.receive_budget {
            let Some(length) = crate::kernel::vm::device::virtio::net::receive_host_stack_frame(
                &mut input,
            ) else {
                break;
            };
            if length == 0 {
                continue;
            }
            changed |= self.process_frame(&input[..length], now_ns);
        }
        self.expire_neighbors(now_ns);
        self.expire_tunnel_peers(now_ns);
        self.fragment_reassembler.expire(now_ns);
        self.retry_pending_arp_requests(now_ns);

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
        self.flush_pending_ipv4_transmissions(now_ns);
        #[cfg(feature = "virtualization")]
        self.flush_host_transmissions();
        if changed {
            let _ = events.set_readiness(source, EventFlags::READABLE);
        }
        changed
    }

    pub(crate) const fn ipv4_address(&self) -> Ipv4Address {
        self.local_address
    }

    pub(crate) const fn ipv4_prefix_length(&self) -> u8 {
        self.local_prefix_length
    }

    pub(crate) fn set_ipv4_configuration(
        &mut self,
        address: Ipv4Address,
        prefix_length: u8,
    ) -> Result<(), Error> {
        if prefix_length > 32 {
            return Err(Error::Inval);
        }
        self.routes.update_connected_route(
            self.local_address,
            self.local_prefix_length,
            address,
            prefix_length,
        )?;
        self.local_address = address;
        self.local_prefix_length = prefix_length;
        Ok(())
    }

    pub(crate) fn set_default_ttl(&mut self, ttl: u8) -> Result<(), Error> {
        if ttl == 0 {
            return Err(Error::Inval);
        }
        self.default_ttl = ttl;
        Ok(())
    }

    pub(crate) fn set_receive_budget(&mut self, budget: usize) -> Result<(), Error> {
        if budget == 0 {
            return Err(Error::Inval);
        }
        self.receive_budget = budget;
        Ok(())
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

    fn poll_tun_devices(&mut self, now_ns: u64, receive_budget: usize) -> bool {
        let Ok(devices) = tun::open_devices() else {
            return false;
        };
        let mut changed = false;
        for device in devices {
            if !device.is_configured() || !device.is_up() || device.mode() != TunTapMode::Tun {
                continue;
            }
            for _ in 0..receive_budget {
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
        self.process_ipv4_packet(&packet, now_ns, None, Some(device))
    }

    fn process_ipv4_packet(
        &mut self,
        packet: &Ipv4Packet<'_>,
        now_ns: u64,
        ethernet_source: Option<MacAddress>,
        tunnel: Option<&Arc<TunDevice>>,
    ) -> bool {
        if let Some(mac_address) = ethernet_source {
            self.remember_neighbor(packet.source, mac_address, now_ns);
        }
        if let Some(device) = tunnel {
            if !self.remember_tunnel_peer(packet.source, Arc::clone(device), now_ns) {
                return false;
            }
        }
        let local_destination = self.is_local_ipv4_address(packet.destination)
            || (packet.protocol == IPV4_PROTOCOL_UDP
                && self.is_broadcast_address(packet.destination));
        let response_source = if self.is_local_ipv4_address(packet.destination) {
            packet.destination
        } else {
            self.local_address
        };
        if !local_destination {
            let ingress_address = tunnel
                .and_then(|device| device.ipv4_configuration())
                .map_or(self.local_address, |(address, _)| address);
            return self.forward_ipv4_packet(packet, now_ns, ingress_address);
        }
        if packet.more_fragments || packet.fragment_offset != 0 {
            let Ok(Some(reassembled)) = self.fragment_reassembler.insert(packet, now_ns) else {
                return false;
            };
            let Ok(packet) = Ipv4Packet::parse(&reassembled) else {
                return false;
            };
            return self.dispatch_local_ipv4(&packet, now_ns, response_source);
        }
        self.dispatch_local_ipv4(packet, now_ns, response_source)
    }

    fn dispatch_local_ipv4(
        &mut self,
        packet: &Ipv4Packet<'_>,
        now_ns: u64,
        response_source: Ipv4Address,
    ) -> bool {
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
            return self.send_icmp_echo_reply(packet, response_source, now_ns);
        }
        if packet.protocol != IPV4_PROTOCOL_TCP {
            self.send_icmp_error(packet, response_source, now_ns, false, 2);
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
            self.queue_tcp_transmission(transmission, now_ns);
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
        replied
    }

    fn process_ipv4(&mut self, frame: EthernetFrame<'_>, now_ns: u64) -> bool {
        let Ok(packet) = Ipv4Packet::parse(frame.payload) else {
            return false;
        };
        self.process_ipv4_packet(&packet, now_ns, Some(frame.source), None)
    }

    fn forward_ipv4_packet(
        &mut self,
        packet: &Ipv4Packet<'_>,
        now_ns: u64,
        ingress_address: Ipv4Address,
    ) -> bool {
        if !self.ipv4_forwarding_enabled || self.is_broadcast_address(packet.destination) {
            return false;
        }
        let may_send_icmp_error = packet.fragment_offset == 0
            && !(packet.protocol == IPV4_PROTOCOL_ICMP
                && crate::kernel::net::icmp::is_error_message(packet.payload));
        if packet.ttl <= 1 {
            if may_send_icmp_error {
                self.send_icmp_error(packet, ingress_address, now_ns, true, 0);
            }
            return false;
        }
        let Some(packet_length) = packet.header.len().checked_add(packet.payload.len()) else {
            return false;
        };
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
        match self.queue_ipv4_packet(packet.destination, forwarded_packet, now_ns) {
            Ok(()) => true,
            Err(Error::NetUnreach) => {
                if may_send_icmp_error {
                    self.send_icmp_error(packet, ingress_address, now_ns, false, 0);
                }
                false
            }
            Err(Error::MsgSize) if packet.dont_fragment && may_send_icmp_error => {
                self.send_icmp_error(packet, ingress_address, now_ns, false, 4);
                false
            }
            Err(_) => false,
        }
    }

    fn send_icmp_error(
        &mut self,
        packet: &Ipv4Packet<'_>,
        source_address: Ipv4Address,
        now_ns: u64,
        time_exceeded: bool,
        unreachable_code: u8,
    ) {
        let source = packet.source.0;
        let destination = packet.destination.0;
        if packet.fragment_offset != 0
            || source == [0; 4]
            || source == [u8::MAX; 4]
            || (224..=239).contains(&source[0])
            || (224..=239).contains(&destination[0])
            || self.is_broadcast_address(packet.source)
            || self.is_broadcast_address(packet.destination)
            || (packet.protocol == IPV4_PROTOCOL_ICMP
                && crate::kernel::net::icmp::is_error_message(packet.payload))
        {
            return;
        }
        let mut icmp = [0; 8 + 60 + 8];
        let next_hop_mtu = (!time_exceeded && unreachable_code == 4).then(|| {
            self.egress_mtu(packet.destination, now_ns)
                .min(usize::from(u16::MAX)) as u16
        });
        let icmp_length = if time_exceeded {
            write_time_exceeded(packet.header, packet.payload, &mut icmp)
        } else if let Some(mtu) = next_hop_mtu {
            write_fragmentation_needed(packet.header, packet.payload, mtu, &mut icmp)
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
            self.default_ttl,
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
        let _queued = self.queue_ipv4_packet_from_source(
            source_address,
            packet.source,
            response,
            now_ns,
        );
    }

    fn queue_ipv4_packet(
        &mut self,
        destination: Ipv4Address,
        packet: Vec<u8>,
        now_ns: u64,
    ) -> Result<(), Error> {
        let broadcast = self.is_broadcast_address(destination);
        if broadcast && destination != Ipv4Address([255; 4]) {
            if self.routes.uses_tunnel(destination) {
                let device = self
                    .routes
                    .tunnel_device(destination)
                    .ok_or(Error::NetDown)?;
                return self.inject_tun_ipv4(&device, &packet);
            }
        }
        if !broadcast {
            if let Some(device) = self.tunnel_peer(destination, now_ns) {
                return self.inject_tun_ipv4(&device, &packet);
            }
            if let Some(device) = self.routes.tunnel_device(destination) {
                return self.inject_tun_ipv4(&device, &packet);
            }
        }
        let next_hop = if broadcast {
            None
        } else {
            Some(self.next_hop(destination).ok_or(Error::NetUnreach)?)
        };
        let mtu = self.frame_size.saturating_sub(ETHERNET_HEADER_LENGTH);
        let packets = if packet.len() > mtu {
            let parsed = Ipv4Packet::parse(&packet).map_err(|_| Error::Proto)?;
            fragment_ipv4_packet(&parsed, mtu)?
        } else {
            let mut packets = Vec::new();
            packets.try_reserve_exact(1).map_err(|_| Error::NoMem)?;
            packets.push(packet);
            packets
        };
        if broadcast {
            for packet in packets {
                self.transmit_ipv4_packet(&packet, MacAddress::BROADCAST)?;
            }
            return Ok(());
        }
        let next_hop = next_hop.ok_or(Error::NetUnreach)?;
        if let Some(mac_address) = self.neighbor_mac(next_hop, now_ns) {
            for packet in packets {
                self.transmit_ipv4_packet(&packet, mac_address)?;
            }
            return Ok(());
        }
        let queued_bytes = packets.iter().try_fold(0usize, |total, packet| {
            total.checked_add(packet.len())
        }).ok_or(Error::MsgSize)?;
        if !self.can_queue_pending_route(packets.len(), queued_bytes) {
            return Err(Error::NoBufs);
        }
        if self.pending_ipv4_transmissions.try_reserve(packets.len()).is_err() {
            return Err(Error::NoMem);
        }
        for packet in packets {
            self.pending_ipv4_transmissions.push_back(PendingIpv4Transmission {
                next_hop,
                destination,
                packet,
            });
        }
        self.pending_route_bytes += queued_bytes;
        self.request_arp(next_hop, now_ns);
        Ok(())
    }

    fn inject_tun_ipv4(&self, device: &TunDevice, packet: &[u8]) -> Result<(), Error> {
        if packet.len() <= TUN_INTERFACE_MTU {
            return device.inject_packet(packet).map_err(|_| Error::Io);
        }
        let parsed = Ipv4Packet::parse(packet).map_err(|_| Error::Proto)?;
        for fragment in fragment_ipv4_packet(&parsed, TUN_INTERFACE_MTU)? {
            device.inject_packet(&fragment).map_err(|_| Error::Io)?;
        }
        Ok(())
    }

    fn egress_mtu(&self, destination: Ipv4Address, now_ns: u64) -> usize {
        let broadcast = self.is_broadcast_address(destination);
        if (broadcast
            && destination != Ipv4Address([255; 4])
            && self.routes.tunnel_device(destination).is_some())
            || (!broadcast
                && (self.tunnel_peer(destination, now_ns).is_some()
                    || self.routes.tunnel_device(destination).is_some()))
        {
            TUN_INTERFACE_MTU
        } else {
            self.frame_size.saturating_sub(ETHERNET_HEADER_LENGTH)
        }
    }

    fn queue_ipv4_packet_from_source(
        &mut self,
        source: Ipv4Address,
        destination: Ipv4Address,
        packet: Vec<u8>,
        now_ns: u64,
    ) -> Result<(), Error> {
        if let Some(device) = self.tunnel_device_for_address(source) {
            return self.inject_tun_ipv4(&device, &packet);
        }
        if source != self.local_address {
            return Err(Error::NetDown);
        }
        self.queue_ipv4_packet(destination, packet, now_ns)
    }

    fn queue_tcp_transmission(&mut self, transmission: TcpTransmission, now_ns: u64) {
        let Some(mut packet) = allocate_zeroed(
            self.frame_size.saturating_sub(ETHERNET_HEADER_LENGTH),
        ) else {
            return;
        };
        let tcp_length = match write_segment(
            &mut packet[IPV4_MINIMUM_HEADER_LENGTH..],
            transmission.source_address,
            transmission.destination_address,
            transmission.source_port,
            transmission.destination_port,
            transmission.sequence_number,
            transmission.acknowledgment_number,
            transmission.flags,
            transmission.window,
            &transmission.payload,
        ) {
            Ok(length) => length,
            Err(_) => return,
        };
        let ipv4_length = match write_ipv4_header(
            &mut packet,
            transmission.source_address,
            transmission.destination_address,
            IPV4_PROTOCOL_TCP,
            self.default_ttl,
            self.next_ipv4_identification,
            0,
            tcp_length,
        ) {
            Ok(length) => length,
            Err(_) => return,
        };
        self.next_ipv4_identification = self.next_ipv4_identification.wrapping_add(1);
        packet.truncate(ipv4_length);
        let _queued = self.queue_ipv4_packet_from_source(
            transmission.source_address,
            transmission.destination_address,
            packet,
            now_ns,
        );
    }

    pub(crate) fn send_udp(
        &mut self,
        handle: crate::kernel::net::udp::UdpSocketHandle,
        destination: Ipv4Address,
        destination_port: u16,
        payload: &[u8],
        now_ns: u64,
    ) -> Result<usize, Error> {
        let maximum_payload_length = usize::from(u16::MAX)
            .saturating_sub(IPV4_MINIMUM_HEADER_LENGTH + UDP_HEADER_LENGTH);
        if payload.len() > maximum_payload_length {
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
        self.queue_udp_transmission(transmission, now_ns)?;
        Ok(length)
    }

    fn queue_udp_transmission(
        &mut self,
        transmission: UdpTransmission,
        now_ns: u64,
    ) -> Result<(), Error> {
        let packet_length = IPV4_MINIMUM_HEADER_LENGTH
            .checked_add(UDP_HEADER_LENGTH)
            .and_then(|length| length.checked_add(transmission.payload.len()))
            .ok_or(Error::MsgSize)?;
        let Some(mut packet) = allocate_zeroed(packet_length) else {
            return Err(Error::NoMem);
        };
        let udp_length = write_datagram(
            &mut packet[IPV4_MINIMUM_HEADER_LENGTH..],
            transmission.source_address,
            transmission.destination_address,
            transmission.source_port,
            transmission.destination_port,
            &transmission.payload,
        )
        .map_err(|_| Error::MsgSize)?;
        let ipv4_length = write_ipv4_header(
            &mut packet,
            transmission.source_address,
            transmission.destination_address,
            IPV4_PROTOCOL_UDP,
            self.default_ttl,
            self.next_ipv4_identification,
            0,
            udp_length,
        )
        .map_err(|_| Error::MsgSize)?;
        self.next_ipv4_identification = self.next_ipv4_identification.wrapping_add(1);
        packet.truncate(ipv4_length);
        self.queue_ipv4_packet_from_source(
            transmission.source_address,
            transmission.destination_address,
            packet,
            now_ns,
        )
    }

    fn flush_pending_ipv4_transmissions(&mut self, now_ns: u64) {
        let pending_count = self.pending_ipv4_transmissions.len();
        for _ in 0..pending_count {
            let Some(mut pending) = self.pending_ipv4_transmissions.pop_front() else {
                break;
            };
            let packet_bytes = pending.packet.len();
            if self.is_broadcast_address(pending.destination) {
                self.pending_route_bytes =
                    self.pending_route_bytes.saturating_sub(packet_bytes);
                if pending.destination != Ipv4Address([255; 4]) {
                    if self.routes.uses_tunnel(pending.destination) {
                        if let Some(device) = self.routes.tunnel_device(pending.destination) {
                            let _ = self.inject_tun_ipv4(&device, &pending.packet);
                        }
                        continue;
                    }
                }
                let _ = self.transmit_ipv4_packet(&pending.packet, MacAddress::BROADCAST);
                continue;
            }
            if let Some(device) = self.tunnel_peer(pending.destination, now_ns) {
                self.pending_route_bytes =
                    self.pending_route_bytes.saturating_sub(packet_bytes);
                let _ = self.inject_tun_ipv4(&device, &pending.packet);
                continue;
            }
            if let Some(device) = self.routes.tunnel_device(pending.destination) {
                self.pending_route_bytes =
                    self.pending_route_bytes.saturating_sub(packet_bytes);
                let _ = self.inject_tun_ipv4(&device, &pending.packet);
                continue;
            }
            let Some(next_hop) = self.next_hop(pending.destination) else {
                self.pending_route_bytes =
                    self.pending_route_bytes.saturating_sub(packet_bytes);
                continue;
            };
            pending.next_hop = next_hop;
            if let Some(mac_address) = self.neighbor_mac(next_hop, now_ns) {
                self.pending_route_bytes =
                    self.pending_route_bytes.saturating_sub(packet_bytes);
                let _ = self.transmit_ipv4_packet(&pending.packet, mac_address);
            } else {
                self.pending_ipv4_transmissions.push_back(pending);
                self.request_arp(next_hop, now_ns);
            }
        }
    }

    fn send_icmp_echo_reply(
        &mut self,
        packet: &Ipv4Packet<'_>,
        response_source: Ipv4Address,
        now_ns: u64,
    ) -> bool {
        let Some(response_length) = IPV4_MINIMUM_HEADER_LENGTH.checked_add(packet.payload.len()) else {
            return false;
        };
        let Some(mut response) = allocate_zeroed(response_length) else {
            return false;
        };
        let Ok(icmp_length) = write_echo_reply(
            packet.payload,
            &mut response[IPV4_MINIMUM_HEADER_LENGTH..],
        ) else {
            return false;
        };
        let Ok(ipv4_length) = write_ipv4_header(
            &mut response,
            response_source,
            packet.source,
            IPV4_PROTOCOL_ICMP,
            self.default_ttl,
            self.next_ipv4_identification,
            0,
            icmp_length,
        ) else {
            return false;
        };
        self.next_ipv4_identification = self.next_ipv4_identification.wrapping_add(1);
        response.truncate(ipv4_length);
        self.queue_ipv4_packet_from_source(response_source, packet.source, response, now_ns)
            .is_ok()
    }

    fn transmit_ipv4_packet(
        &mut self,
        packet: &[u8],
        destination_mac: MacAddress,
    ) -> Result<(), Error> {
        if packet.len() > self.frame_size.saturating_sub(ETHERNET_HEADER_LENGTH) {
            return Err(Error::MsgSize);
        }
        let frame_length = ETHERNET_HEADER_LENGTH
            .checked_add(packet.len())
            .ok_or(Error::MsgSize)?;
        let mut frame = allocate_zeroed(frame_length).ok_or(Error::NoMem)?;
        if write_ethernet_header(
            &mut frame[..ETHERNET_HEADER_LENGTH],
            destination_mac,
            self.mac_address,
            ETHER_TYPE_IPV4,
        )
        .is_err()
        {
            return Err(Error::Proto);
        }
        frame[ETHERNET_HEADER_LENGTH..].copy_from_slice(packet);
        #[cfg(feature = "virtualization")]
        crate::kernel::vm::device::virtio::net::host_transmit_frame(&frame);
        #[cfg(not(feature = "virtualization"))]
        self.device.transmit_frame(&frame)?;
        Ok(())
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

    fn request_arp(&mut self, address: Ipv4Address, now_ns: u64) {
        if self.arp_requests.iter().any(|request| request.address == address) {
            return;
        }
        if self.arp_requests.len() >= MAX_PENDING_ROUTE_ENTRIES
            || self.arp_requests.try_reserve(1).is_err()
        {
            return;
        }
        self.arp_requests.push(PendingArpRequest {
            address,
            next_attempt_ns: now_ns.saturating_add(ARP_RETRY_INTERVAL_NS),
            attempts: 1,
        });
        self.send_arp_request(address);
    }

    fn retry_pending_arp_requests(&mut self, now_ns: u64) {
        let mut index = 0;
        while index < self.arp_requests.len() {
            let request = &self.arp_requests[index];
            if !self
                .pending_ipv4_transmissions
                .iter()
                .any(|pending| pending.next_hop == request.address)
            {
                self.arp_requests.remove(index);
                continue;
            }
            if now_ns < request.next_attempt_ns {
                index += 1;
                continue;
            }
            if request.attempts < MAX_ARP_REQUEST_ATTEMPTS {
                let address = request.address;
                let request = &mut self.arp_requests[index];
                request.attempts += 1;
                let shift = u32::from(request.attempts.saturating_sub(1)).min(3);
                let retry_delay = ARP_RETRY_INTERVAL_NS
                    .saturating_mul(1u64 << shift)
                    .min(MAX_ARP_RETRY_INTERVAL_NS);
                request.next_attempt_ns = now_ns.saturating_add(retry_delay);
                self.send_arp_request(address);
                index += 1;
                continue;
            }

            let address = self.arp_requests.remove(index).address;
            let mut dropped_bytes = 0usize;
            let pending_count = self.pending_ipv4_transmissions.len();
            for _ in 0..pending_count {
                let Some(mut pending) = self.pending_ipv4_transmissions.pop_front() else {
                    break;
                };
                if pending.next_hop != address {
                    self.pending_ipv4_transmissions.push_back(pending);
                    continue;
                }
                let has_alternate_egress = self.tunnel_peer(pending.destination, now_ns).is_some()
                    || self.routes.tunnel_device(pending.destination).is_some()
                    || self.is_broadcast_address(pending.destination);
                let routed_next_hop = self.next_hop(pending.destination);
                let changed_next_hop = routed_next_hop.filter(|next_hop| *next_hop != address);
                if has_alternate_egress || changed_next_hop.is_some() {
                    pending.next_hop = routed_next_hop.unwrap_or(address);
                    self.pending_ipv4_transmissions.push_back(pending);
                    if let Some(next_hop) = changed_next_hop {
                        self.request_arp(next_hop, now_ns);
                    }
                } else {
                    dropped_bytes = dropped_bytes.saturating_add(pending.packet.len());
                }
            }
            self.pending_route_bytes = self.pending_route_bytes.saturating_sub(dropped_bytes);
        }
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
        #[cfg(feature = "virtualization")]
        {
            crate::kernel::vm::device::virtio::net::host_transmit_frame(&frame);
            true
        }
        #[cfg(not(feature = "virtualization"))]
        {
            self.device.transmit_frame(&frame).is_ok()
        }
    }

    fn remember_neighbor(&mut self, address: Ipv4Address, mac: MacAddress, now_ns: u64) {
        if address == Ipv4Address([0; 4]) || mac.is_multicast() {
            return;
        }
        self.expire_neighbors(now_ns);
        let expires_at_ns = now_ns.saturating_add(NEIGHBOR_LIFETIME_NS);
        if let Some(entry) = self
            .neighbors
            .iter_mut()
            .find(|entry| entry.address == address)
        {
            entry.mac = mac;
            entry.expires_at_ns = expires_at_ns;
            self.arp_requests.retain(|request| request.address != address);
            return;
        }
        if self.neighbors.len() >= MAX_NEIGHBORS {
            let Some(index) = self
                .neighbors
                .iter()
                .enumerate()
                .min_by_key(|(_, entry)| entry.expires_at_ns)
                .map(|(index, _)| index)
            else {
                return;
            };
            self.neighbors[index] = NeighborEntry {
                address,
                mac,
                expires_at_ns,
            };
        } else {
            if self.neighbors.try_reserve(1).is_err() {
                return;
            }
            self.neighbors.push(NeighborEntry {
                address,
                mac,
                expires_at_ns,
            });
        }
        self.arp_requests.retain(|request| request.address != address);
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
    ) -> bool {
        self.expire_tunnel_peers(now_ns);
        let expires_at_ns = now_ns.saturating_add(NEIGHBOR_LIFETIME_NS);
        if let Some(peer) = self
            .tunnel_peers
            .iter_mut()
            .find(|peer| peer.address == address)
        {
            peer.device = device;
            peer.expires_at_ns = expires_at_ns;
            return true;
        }
        if self.tunnel_peers.len() >= MAX_TUNNEL_PEERS {
            if let Some(index) = self
                .tunnel_peers
                .iter()
                .enumerate()
                .min_by_key(|(_, peer)| peer.expires_at_ns)
                .map(|(index, _)| index)
            {
                self.tunnel_peers[index] = TunnelPeer {
                    address,
                    device,
                    expires_at_ns,
                };
                return true;
            }
            return false;
        }
        if self.tunnel_peers.try_reserve(1).is_err() {
            return false;
        }
        self.tunnel_peers.push(TunnelPeer {
            address,
            device,
            expires_at_ns,
        });
        true
    }

    fn tunnel_peer(&self, address: Ipv4Address, now_ns: u64) -> Option<Arc<TunDevice>> {
        self.tunnel_peers
            .iter()
            .find(|peer| {
                peer.address == address
                    && peer.expires_at_ns > now_ns
                    && peer.device.mode() == TunTapMode::Tun
                    && peer.device.is_configured()
                    && peer.device.is_up()
            })
            .map(|peer| Arc::clone(&peer.device))
    }

    fn tunnel_device_for_address(&self, address: Ipv4Address) -> Option<Arc<TunDevice>> {
        tun::open_devices().ok()?.into_iter().find(|device| {
            device.mode() == TunTapMode::Tun
                && device.is_configured()
                && device.is_up()
                && device
                    .ipv4_configuration()
                    .is_some_and(|(interface_address, _)| interface_address == address)
        })
    }

    fn expire_tunnel_peers(&mut self, now_ns: u64) {
        self.tunnel_peers
            .retain(|peer| peer.expires_at_ns > now_ns);
    }

    fn can_queue_pending_route(&self, entries: usize, payload_bytes: usize) -> bool {
        self.pending_ipv4_transmissions.len().saturating_add(entries)
            <= MAX_PENDING_ROUTE_ENTRIES
            && self
                .pending_route_bytes
                .checked_add(payload_bytes)
                .is_some_and(|bytes| bytes <= MAX_PENDING_ROUTE_BYTES)
    }

    #[cfg(feature = "virtualization")]
    fn flush_host_transmissions(&mut self) {
        let Some(mut frame) = allocate_zeroed(self.frame_size) else {
            return;
        };
        for _ in 0..self.receive_budget {
            let Some(length) =
                crate::kernel::vm::device::virtio::net::peek_host_transmit_frame(&mut frame)
            else {
                break;
            };
            if length == 0 {
                continue;
            }
            match self.device.transmit_frame(&frame[..length]) {
                Ok(()) => {
                    let _consumed =
                        crate::kernel::vm::device::virtio::net::consume_host_transmit_frame(length);
                }
                Err(Error::Busy) => break,
                Err(_) => {
                    let _dropped =
                        crate::kernel::vm::device::virtio::net::drop_host_transmit_frame(length);
                }
            }
        }
    }

    fn next_hop(&self, destination: Ipv4Address) -> Option<Ipv4Address> {
        self.routes.next_hop(destination)
    }

    pub(crate) fn is_broadcast_address(&self, address: Ipv4Address) -> bool {
        self.routes.is_broadcast_address(address)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Ipv4Address, Ipv4RouteTable, TunDevice, TunTapMode, INITIAL_DEFAULT_GATEWAY,
        INITIAL_LOCAL_ADDRESS, INITIAL_PREFIX_LENGTH,
    };

    #[test]
    fn route_lookup_prefers_the_longest_prefix() {
        let mut routes = Ipv4RouteTable::new(
            INITIAL_LOCAL_ADDRESS,
            INITIAL_PREFIX_LENGTH,
            INITIAL_DEFAULT_GATEWAY,
        )
        .unwrap();
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
            Some(INITIAL_DEFAULT_GATEWAY)
        );
    }

    #[test]
    fn deleted_default_route_stops_fallback_routing() {
        let mut routes = Ipv4RouteTable::new(
            INITIAL_LOCAL_ADDRESS,
            INITIAL_PREFIX_LENGTH,
            INITIAL_DEFAULT_GATEWAY,
        )
        .unwrap();
        routes.remove(Ipv4Address([0; 4]), 0).unwrap();
        assert_eq!(routes.next_hop(Ipv4Address([203, 0, 113, 1])), None);
    }

    #[test]
    fn directed_broadcast_lookup_uses_connected_prefix_routes() {
        let mut routes = Ipv4RouteTable::new(
            INITIAL_LOCAL_ADDRESS,
            INITIAL_PREFIX_LENGTH,
            INITIAL_DEFAULT_GATEWAY,
        )
        .unwrap();
        assert!(routes.is_broadcast_address(Ipv4Address([10, 0, 2, 255])));
        assert!(!routes.is_broadcast_address(Ipv4Address([10, 0, 2, 254])));
        routes.add(Ipv4Address([10, 0, 0, 0]), 8, None).unwrap();
        routes
            .add(
                Ipv4Address([10, 255, 0, 0]),
                16,
                Some(Ipv4Address([192, 0, 2, 1])),
            )
            .unwrap();
        assert!(!routes.is_broadcast_address(Ipv4Address([10, 255, 255, 255])));
    }

    #[test]
    fn directed_broadcast_retains_tunnel_route_when_device_is_down() {
        let mut routes = Ipv4RouteTable::new(
            INITIAL_LOCAL_ADDRESS,
            INITIAL_PREFIX_LENGTH,
            INITIAL_DEFAULT_GATEWAY,
        )
        .unwrap();
        let device = TunDevice::new().unwrap();
        device.configure(TunTapMode::Tun).unwrap();
        device.set_ipv4_address(Ipv4Address([198, 51, 100, 1]));
        device.set_ipv4_prefix_length(24);
        device.set_up(true);
        routes
            .add_tunnel(Ipv4Address([198, 51, 100, 0]), 24, device.clone())
            .unwrap();

        let broadcast = Ipv4Address([198, 51, 100, 255]);
        assert!(routes.is_broadcast_address(broadcast));
        assert!(routes.uses_tunnel(broadcast));
        assert!(routes.tunnel_device(broadcast).is_some());

        device.set_up(false);
        assert!(routes.is_broadcast_address(broadcast));
        assert!(routes.uses_tunnel(broadcast));
        assert!(routes.tunnel_device(broadcast).is_none());
    }
}
