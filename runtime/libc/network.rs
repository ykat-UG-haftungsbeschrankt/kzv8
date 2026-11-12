/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_int;
use core::ptr;
use core::sync::atomic::{AtomicU16, Ordering};

use crate::kernel::error::Error;
use crate::kernel::net::tcp::connection::{TcpSocketHandle, TcpState};
use crate::kernel::net::udp::UdpSocketHandle;
use crate::kernel::net::wire::Ipv4Address;
use crate::kernel::runtime::runtime_ffi::{
    with_active_tcp_context, TcpContext,
};
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::vfs::plugin::tun::{TunDevice, TunTapMode};

pub(crate) const SOCKET_FD_BASE: i32 = 128;
const SOCKET_COUNT: usize = 32;
const AF_INET: i32 = 2;
const SOCK_STREAM: i32 = 1;
const SOCK_DGRAM: i32 = 2;
const SOCK_RAW: i32 = 3;
const SOCK_NONBLOCK: i32 = 0x0000_0800;
const SOCK_CLOEXEC: i32 = 0x0008_0000;
const SOCK_CLOFORK: i32 = 0x0010_0000;
const IPPROTO_TCP: i32 = 6;
const IPPROTO_UDP: i32 = 17;
const SOL_SOCKET: i32 = 1;
const SO_REUSEADDR: i32 = 2;
const SO_TYPE: i32 = 3;
const SO_ERROR: i32 = 4;
const SO_BROADCAST: i32 = 6;
const SO_SNDBUF: i32 = 7;
const SO_RCVBUF: i32 = 8;
const SO_ACCEPTCONN: i32 = 30;
const SO_PROTOCOL: i32 = 38;
const SO_DOMAIN: i32 = 39;
const SOCKET_BUFFER_SIZE: i32 = 4096;
const O_NONBLOCK: i32 = 0o4000;
const PACKET_FD_BASE: i32 = 256;
const PACKET_DEVICE_COUNT: usize = 16;
const TUNSETIFF: usize = 0x4004_54ca;
const TUNGETIFF: usize = 0x8004_54d2;
const SIOCGIFFLAGS: usize = 0x8913;
const SIOCSIFFLAGS: usize = 0x8914;
const SIOCGIFADDR: usize = 0x8915;
const SIOCSIFADDR: usize = 0x8916;
const SIOCGIFNETMASK: usize = 0x891b;
const SIOCSIFNETMASK: usize = 0x891c;
const SIOCGIFMTU: usize = 0x8921;
const SIOCGIFHWADDR: usize = 0x8927;
const IFF_TUN: u16 = 0x0001;
const IFF_TAP: u16 = 0x0002;
const IFF_NO_PI: u16 = 0x1000;
const IFF_UP: i16 = 0x0001;
const IFF_RUNNING: i16 = 0x0040;
const ARPHRD_ETHER: u16 = 1;
const DEFAULT_INTERFACE_MTU: i32 = 1500;
const ETHERNET_HEADER_LENGTH: usize = 14;
const TUN_PI_LENGTH: usize = 4;
const ETHER_TYPE_IPV4: u16 = 0x0800;
const FIRST_EPHEMERAL_PORT: u16 = 49_152;
const EPHEMERAL_PORT_COUNT: u16 = 16_384;
const CONNECT_TIMEOUT_NS: u64 = 30_000_000_000;
const IO_POLL_LIMIT: usize = 100_000;

static NEXT_EPHEMERAL_PORT: AtomicU16 = AtomicU16::new(0);
static SOCKETS: IrqSpinLock<[Option<SocketDescriptor>; SOCKET_COUNT]> =
    IrqSpinLock::new([None; SOCKET_COUNT]);
static PACKET_DEVICES: IrqSpinLock<[Option<PacketDescriptor>; PACKET_DEVICE_COUNT]> =
    IrqSpinLock::new([const { None }; PACKET_DEVICE_COUNT]);

#[derive(Clone, Copy, Eq, PartialEq)]
enum SocketState {
    Created,
    Bound,
    Connecting,
    Connected,
    Listening,
    Failed,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SocketType {
    Stream,
    Datagram,
}

#[derive(Clone, Copy)]
struct SocketDescriptor {
    socket_type: SocketType,
    state: SocketState,
    local_address: Ipv4Address,
    local_port: u16,
    remote_address: Ipv4Address,
    remote_port: u16,
    handle: Option<TcpSocketHandle>,
    udp_handle: Option<UdpSocketHandle>,
    nonblocking: bool,
    read_shutdown: bool,
    write_shutdown: bool,
    reuse_address: bool,
    broadcast: bool,
}

#[derive(Clone)]
struct PacketDescriptor {
    device: Arc<TunDevice>,
    mode: Option<TunTapMode>,
    no_packet_info: bool,
    nonblocking: bool,
    name: [u8; 16],
}

#[repr(C)]
struct TunIfreq {
    name: [u8; 16],
    flags: i16,
    padding: [u8; 22],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SockaddrIn {
    family: u16,
    port: u16,
    address: u32,
}

pub(crate) fn is_socket_fd(file_descriptor: i32) -> bool {
    socket_index(file_descriptor).is_ok()
}

pub(crate) fn has_socket_fd(file_descriptor: i32) -> bool {
    socket_index(file_descriptor)
        .ok()
        .and_then(|index| SOCKETS.lock()[index])
        .is_some()
}

pub(crate) fn is_packet_fd(file_descriptor: i32) -> bool {
    packet_index(file_descriptor).is_ok()
}

pub(crate) fn has_packet_fd(file_descriptor: i32) -> bool {
    packet_index(file_descriptor)
        .ok()
        .and_then(|index| PACKET_DEVICES.lock()[index].as_ref().map(|_| ()))
        .is_some()
}

pub(crate) fn interface_list() -> Result<Vec<(u32, [u8; 16])>, Error> {
    let host_is_active = with_active_tcp_context(|context| context.local_ipv4().is_some())
        .unwrap_or(false);
    let mut interfaces = Vec::new();
    interfaces
        .try_reserve_exact(PACKET_DEVICE_COUNT + usize::from(host_is_active))
        .map_err(|_| Error::NoMem)?;
    if host_is_active {
        let mut name = [0; 16];
        name[..4].copy_from_slice(b"eth0");
        interfaces.push((1, name));
    }
    let devices = PACKET_DEVICES.lock();
    for (index, descriptor) in devices.iter().enumerate() {
        let Some(descriptor) = descriptor else {
            continue;
        };
        if descriptor.mode.is_some() && descriptor.name[0] != 0 {
            interfaces.push((index as u32 + 2, descriptor.name));
        }
    }
    Ok(interfaces)
}

pub(crate) fn tun_device_by_name(name: &[u8]) -> Result<Arc<TunDevice>, Error> {
    let devices = PACKET_DEVICES.lock();
    devices
        .iter()
        .filter_map(Option::as_ref)
        .find(|descriptor| {
            if descriptor.mode != Some(TunTapMode::Tun) || !descriptor.device.is_up() {
                return false;
            }
            let length = descriptor
                .name
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(descriptor.name.len());
            &descriptor.name[..length] == name
        })
        .map(|descriptor| Arc::clone(&descriptor.device))
        .ok_or(Error::NoDev)
}

pub(crate) fn open_tun_device(nonblocking: bool) -> Result<i32, Error> {
    let device = TunDevice::new()?;
    let mut devices = PACKET_DEVICES.lock();
    let index = devices
        .iter()
        .position(Option::is_none)
        .ok_or(Error::Mfile)?;
    devices[index] = Some(PacketDescriptor {
        device,
        mode: None,
        no_packet_info: false,
        nonblocking,
        name: [0; 16],
    });
    Ok(PACKET_FD_BASE + index as i32)
}

pub(crate) fn tun_ioctl(
    file_descriptor: i32,
    request: usize,
    argument: *mut c_void,
) -> Result<(), Error> {
    let index = packet_index(file_descriptor)?;
    if argument.is_null() {
        return Err(Error::Fault);
    }
    match request {
        TUNSETIFF => {
            let mut ifreq = unsafe { ptr::read_unaligned(argument.cast::<TunIfreq>()) };
            let flags = ifreq.flags as u16;
            let mode = match flags & (IFF_TUN | IFF_TAP) {
                IFF_TUN => TunTapMode::Tun,
                IFF_TAP => TunTapMode::Tap,
                _ => return Err(Error::Inval),
            };
            if flags & !(IFF_TUN | IFF_TAP | IFF_NO_PI) != 0 {
                return Err(Error::Inval);
            }
            let mut name = ifreq.name;
            if name[0] == 0 {
                name = default_interface_name(mode, index);
            } else if !name.contains(&0) {
                return Err(Error::NameTooLong);
            }
            let mut devices = PACKET_DEVICES.lock();
            if devices.iter().enumerate().any(|(other_index, other)| {
                other_index != index
                    && other.as_ref().is_some_and(|other| {
                        other.mode.is_some() && other.name == name
                    })
            }) {
                return Err(Error::Exist);
            }
            let descriptor = devices[index].as_mut().ok_or(Error::BadFd)?;
            descriptor.device.configure(mode)?;
            descriptor.mode = Some(mode);
            descriptor.no_packet_info = flags & IFF_NO_PI != 0;
            descriptor.name = name;
            ifreq.name = name;
            unsafe { ptr::write_unaligned(argument.cast::<TunIfreq>(), ifreq) };
            Ok(())
        }
        TUNGETIFF => {
            let devices = PACKET_DEVICES.lock();
            let descriptor = devices[index].as_ref().ok_or(Error::BadFd)?;
            let mode = descriptor.mode.ok_or(Error::Inval)?;
            let flags = match mode {
                TunTapMode::Tun => IFF_TUN,
                TunTapMode::Tap => IFF_TAP,
            } | if descriptor.no_packet_info { IFF_NO_PI } else { 0 };
            let ifreq = TunIfreq {
                name: descriptor.name,
                flags: flags as i16,
                padding: [0; 22],
            };
            unsafe { ptr::write_unaligned(argument.cast::<TunIfreq>(), ifreq) };
            Ok(())
        }
        _ => Err(Error::NoSys),
    }
}

pub(crate) fn interface_ioctl(
    request: usize,
    argument: *mut c_void,
) -> Result<(), Error> {
    if argument.is_null() {
        return Err(Error::Fault);
    }
    let mut ifreq = unsafe { ptr::read_unaligned(argument.cast::<TunIfreq>()) };
    if &ifreq.name[..5] == b"eth0\0" {
        return host_interface_ioctl(request, argument, ifreq);
    }
    let index = packet_interface_index(ifreq.name)?;
    let devices = PACKET_DEVICES.lock();
    let descriptor = devices[index].as_ref().ok_or(Error::NoDev)?;
    let device = Arc::clone(&descriptor.device);
    drop(devices);
    match request {
        SIOCGIFFLAGS => {
            ifreq.flags = if device.is_up() {
                IFF_UP | IFF_RUNNING
            } else {
                0
            };
            unsafe { ptr::write_unaligned(argument.cast::<TunIfreq>(), ifreq) };
            Ok(())
        }
        SIOCSIFFLAGS => {
            if ifreq.flags & !(IFF_UP | IFF_RUNNING) != 0 {
                return Err(Error::Inval);
            }
            device.set_up(ifreq.flags & IFF_UP != 0);
            Ok(())
        }
        SIOCGIFADDR | SIOCGIFNETMASK => {
            let (address, prefix_length) = device
                .ipv4_configuration()
                .unwrap_or((Ipv4Address([0; 4]), 0));
            let address = if request == SIOCGIFADDR {
                address
            } else {
                Ipv4Address(mask_from_prefix(prefix_length))
            };
            let sockaddr = SockaddrIn {
                family: AF_INET as u16,
                port: 0,
                address: u32::from_ne_bytes(address.0),
            };
            unsafe {
                ptr::write_unaligned(argument.cast::<u8>().add(16).cast::<SockaddrIn>(), sockaddr)
            };
            Ok(())
        }
        SIOCSIFADDR | SIOCSIFNETMASK => {
            let sockaddr = unsafe {
                ptr::read_unaligned(argument.cast::<u8>().add(16).cast::<SockaddrIn>())
            };
            if i32::from(sockaddr.family) != AF_INET {
                return Err(Error::AfNoSupport);
            }
            let address = Ipv4Address(sockaddr.address.to_ne_bytes());
            if request == SIOCSIFADDR {
                validate_interface_address(address, Some(index))?;
                device.set_ipv4_address(address);
                Ok(())
            } else {
                let Some(prefix_length) = prefix_from_mask(address.0) else {
                    return Err(Error::Inval);
                };
                device.set_ipv4_prefix_length(prefix_length);
                Ok(())
            }
        }
        SIOCGIFMTU => {
            unsafe {
                ptr::write_unaligned(argument.cast::<u8>().add(16).cast::<i32>(), DEFAULT_INTERFACE_MTU)
            };
            Ok(())
        }
        SIOCGIFHWADDR => {
            unsafe {
                let output = argument.cast::<u8>().add(16);
                ptr::write_unaligned(output.cast::<u16>(), ARPHRD_ETHER);
                ptr::write_bytes(output.add(2), 0, 14);
                ptr::copy_nonoverlapping(device.mac_address().as_ptr(), output.add(2), 6);
            }
            Ok(())
        }
        _ => Err(Error::NoSys),
    }
}

fn host_interface_ioctl(
    request: usize,
    argument: *mut c_void,
    mut ifreq: TunIfreq,
) -> Result<(), Error> {
    match request {
        SIOCGIFFLAGS => {
            ifreq.flags = IFF_UP | IFF_RUNNING;
            unsafe { ptr::write_unaligned(argument.cast::<TunIfreq>(), ifreq) };
            Ok(())
        }
        SIOCSIFFLAGS => {
            if ifreq.flags & IFF_UP == 0 {
                Err(Error::NoSys)
            } else {
                Ok(())
            }
        }
        SIOCGIFADDR | SIOCGIFNETMASK => {
            let (address, prefix_length) = active_context(|context| {
                (context.local_ipv4(), context.local_ipv4_prefix_length())
            })?;
            let address = address.ok_or(Error::NoDev)?;
            let prefix_length = prefix_length.ok_or(Error::NoDev)?;
            let address = if request == SIOCGIFADDR {
                address
            } else {
                Ipv4Address(mask_from_prefix(prefix_length))
            };
            let sockaddr = SockaddrIn {
                family: AF_INET as u16,
                port: 0,
                address: u32::from_ne_bytes(address.0),
            };
            unsafe {
                ptr::write_unaligned(argument.cast::<u8>().add(16).cast::<SockaddrIn>(), sockaddr)
            };
            Ok(())
        }
        SIOCGIFMTU => {
            let mtu = active_context(|context| context.ethernet_mtu())?
                .ok_or(Error::NoDev)?;
            let mtu = i32::try_from(mtu).map_err(|_| Error::Overflow)?;
            unsafe {
                ptr::write_unaligned(argument.cast::<u8>().add(16).cast::<i32>(), mtu)
            };
            Ok(())
        }
        SIOCGIFHWADDR => {
            let mac_address = active_context(|context| context.ethernet_mac_address())?
                .ok_or(Error::NoDev)?;
            unsafe {
                let output = argument.cast::<u8>().add(16);
                ptr::write_unaligned(output.cast::<u16>(), ARPHRD_ETHER);
                ptr::write_bytes(output.add(2), 0, 14);
                ptr::copy_nonoverlapping(mac_address.as_ptr(), output.add(2), 6);
            }
            Ok(())
        }
        SIOCSIFADDR | SIOCSIFNETMASK => {
            let sockaddr = unsafe {
                ptr::read_unaligned(argument.cast::<u8>().add(16).cast::<SockaddrIn>())
            };
            if i32::from(sockaddr.family) != AF_INET {
                return Err(Error::AfNoSupport);
            }
            let configured_address = Ipv4Address(sockaddr.address.to_ne_bytes());
            let (current_address, current_prefix) = active_context(|context| {
                (context.local_ipv4(), context.local_ipv4_prefix_length())
            })?;
            if request == SIOCSIFADDR {
                validate_interface_address(configured_address, None)?;
                active_context(|context| {
                    context.set_host_ipv4_configuration(
                        configured_address,
                        current_prefix.ok_or(Error::NoDev)?,
                    )
                })??;
            } else {
                let Some(prefix_length) = prefix_from_mask(configured_address.0) else {
                    return Err(Error::Inval);
                };
                active_context(|context| {
                    context.set_host_ipv4_configuration(
                        current_address.ok_or(Error::NoDev)?,
                        prefix_length,
                    )
                })??;
            }
            Ok(())
        }
        _ => Err(Error::NoSys),
    }
}

fn packet_interface_index(name: [u8; 16]) -> Result<usize, Error> {
    PACKET_DEVICES
        .lock()
        .iter()
        .position(|descriptor| {
            descriptor
                .as_ref()
                .is_some_and(|descriptor| descriptor.mode.is_some() && descriptor.name == name)
        })
        .ok_or(Error::NoDev)
}

fn validate_interface_address(
    address: Ipv4Address,
    excluded_packet_index: Option<usize>,
) -> Result<(), Error> {
    if address == Ipv4Address([0; 4]) {
        return Ok(());
    }
    if active_context(|context| context.local_ipv4() == Some(address))? {
        return Err(Error::Exist);
    }
    let devices = PACKET_DEVICES.lock();
    if devices.iter().enumerate().any(|(other_index, descriptor)| {
        Some(other_index) != excluded_packet_index
            && descriptor.as_ref().is_some_and(|descriptor| {
                descriptor.mode.is_some()
                    && descriptor
                        .device
                        .ipv4_configuration()
                        .is_some_and(|(other_address, _)| other_address == address)
            })
    }) {
        return Err(Error::Exist);
    }
    Ok(())
}

fn prefix_from_mask(mask: [u8; 4]) -> Option<u8> {
    let mut prefix_length = 0;
    let mut found_zero = false;
    for byte in mask {
        for bit in (0..8).rev() {
            let set = byte & (1 << bit) != 0;
            if set && found_zero {
                return None;
            }
            if set {
                prefix_length += 1;
            } else {
                found_zero = true;
            }
        }
    }
    Some(prefix_length)
}

fn mask_from_prefix(prefix_length: u8) -> [u8; 4] {
    let mut mask = [0; 4];
    for bit in 0..prefix_length {
        mask[usize::from(bit / 8)] |= 1 << (7 - bit % 8);
    }
    mask
}

pub(crate) fn read_packet_fd(
    file_descriptor: i32,
    output: &mut [u8],
) -> Result<usize, Error> {
    let descriptor = packet_descriptor(file_descriptor)?;
    let mode = descriptor.mode.ok_or(Error::Inval)?;
    if descriptor.no_packet_info {
        return descriptor.device.read_packet(output);
    }
    if output.len() < TUN_PI_LENGTH {
        return Err(Error::MsgSize);
    }
    let length = descriptor.device.read_packet(&mut output[TUN_PI_LENGTH..])?;
    let protocol = match mode {
        TunTapMode::Tun => ETHER_TYPE_IPV4,
        TunTapMode::Tap => {
            if length < ETHERNET_HEADER_LENGTH {
                return Err(Error::Proto);
            }
            u16::from_be_bytes([
                output[TUN_PI_LENGTH + 12],
                output[TUN_PI_LENGTH + 13],
            ])
        }
    };
    output[..2].fill(0);
    output[2..4].copy_from_slice(&protocol.to_be_bytes());
    Ok(length + TUN_PI_LENGTH)
}

pub(crate) fn write_packet_fd(
    file_descriptor: i32,
    packet: &[u8],
) -> Result<usize, Error> {
    let descriptor = packet_descriptor(file_descriptor)?;
    let mode = descriptor.mode.ok_or(Error::Inval)?;
    let (packet, packet_info_protocol) = if descriptor.no_packet_info {
        (packet, None)
    } else {
        if packet.len() < TUN_PI_LENGTH {
            return Err(Error::MsgSize);
        }
        (
            &packet[TUN_PI_LENGTH..],
            Some(u16::from_be_bytes([packet[2], packet[3]])),
        )
    };
    if let Some(protocol) = packet_info_protocol {
        match mode {
            TunTapMode::Tun if protocol != ETHER_TYPE_IPV4 => return Err(Error::Prototype),
            TunTapMode::Tap if packet.len() < ETHERNET_HEADER_LENGTH => {
                return Err(Error::MsgSize);
            }
            TunTapMode::Tap
                if u16::from_be_bytes([packet[12], packet[13]]) != protocol =>
            {
                return Err(Error::Proto);
            }
            _ => {}
        }
    }
    let length = descriptor.device.write_packet(packet)?;
    match mode {
        TunTapMode::Tun => {
            let _progress = with_active_tcp_context(TcpContext::poll_network);
        }
        TunTapMode::Tap => {
            crate::kernel::vm::device::virtio::net::poll_tap_devices();
        }
    }
    Ok(length)
}

pub(crate) fn pump_packet_network() {
    let _progress = with_active_tcp_context(TcpContext::poll_network);
    crate::kernel::vm::device::virtio::net::poll_tap_devices();
}

pub(crate) fn close_packet_fd(file_descriptor: i32) -> Result<(), Error> {
    let index = packet_index(file_descriptor)?;
    let descriptor = PACKET_DEVICES.lock()[index].take().ok_or(Error::BadFd)?;
    descriptor.device.set_up(false);
    Ok(())
}

pub(crate) fn packet_readiness(file_descriptor: i32, events: i16) -> Result<i16, Error> {
    let _progress = with_active_tcp_context(TcpContext::poll_network);
    let descriptor = packet_descriptor(file_descriptor)?;
    if descriptor.mode.is_none() {
        return Err(Error::Inval);
    }
    let mut readiness = 0;
    if events & 0x001 != 0 && descriptor.device.is_readable() {
        readiness |= 0x001;
    }
    if events & 0x004 != 0 && descriptor.device.is_writable() {
        readiness |= 0x004;
    }
    Ok(readiness)
}

pub(crate) fn packet_nonblocking(file_descriptor: i32) -> Result<bool, Error> {
    Ok(packet_descriptor(file_descriptor)?.nonblocking)
}

pub(crate) fn set_packet_nonblocking(
    file_descriptor: i32,
    nonblocking: bool,
) -> Result<(), Error> {
    let index = packet_index(file_descriptor)?;
    let mut devices = PACKET_DEVICES.lock();
    let descriptor = devices[index].as_mut().ok_or(Error::BadFd)?;
    descriptor.nonblocking = nonblocking;
    Ok(())
}

pub(crate) fn create_socket(
    domain: i32,
    socket_type: i32,
    protocol: i32,
) -> Result<i32, Error> {
    if domain != AF_INET {
        return Err(Error::AfNoSupport);
    }
    let nonblocking = socket_type & SOCK_NONBLOCK != 0;
    let socket_type = socket_type & !(SOCK_NONBLOCK | SOCK_CLOEXEC | SOCK_CLOFORK);
    if socket_type == SOCK_RAW {
        return Err(Error::SockTypeNoSupport);
    }
    let socket_type = match socket_type {
        SOCK_STREAM if protocol == 0 || protocol == IPPROTO_TCP => SocketType::Stream,
        SOCK_DGRAM if protocol == 0 || protocol == IPPROTO_UDP => SocketType::Datagram,
        SOCK_STREAM | SOCK_DGRAM if protocol == IPPROTO_TCP || protocol == IPPROTO_UDP => {
            return Err(Error::Prototype);
        }
        SOCK_STREAM | SOCK_DGRAM => return Err(Error::ProtoNoSupport),
        _ => return Err(Error::SockTypeNoSupport),
    };
    let udp_handle = if socket_type == SocketType::Datagram {
        Some(active_context(|context| context.udp_create())??)
    } else {
        None
    };
    let descriptor = SocketDescriptor {
        socket_type,
        state: SocketState::Created,
        local_address: Ipv4Address([0; 4]),
        local_port: 0,
        remote_address: Ipv4Address([0; 4]),
        remote_port: 0,
        handle: None,
        udp_handle,
        nonblocking,
        read_shutdown: false,
        write_shutdown: false,
        reuse_address: false,
        broadcast: false,
    };
    let mut sockets = SOCKETS.lock();
    let index = sockets
        .iter()
        .position(Option::is_none)
        .ok_or(Error::Mfile)?;
    sockets[index] = Some(descriptor);
    Ok(SOCKET_FD_BASE + index as i32)
}

pub(crate) fn bind(
    file_descriptor: i32,
    address: Ipv4Address,
    port: u16,
) -> Result<(), Error> {
    let index = socket_index(file_descriptor)?;
    let mut descriptor = socket_descriptor(index)?;
    if descriptor.state != SocketState::Created {
        return Err(Error::Inval);
    }
    let (local_address, is_local_address) = active_context(|context| {
        (context.local_ipv4(), context.is_local_ipv4_address(address))
    })?;
    let local_address = local_address.ok_or(Error::NoDev)?;
    if address != Ipv4Address([0; 4]) && !is_local_address {
        return Err(Error::Inval);
    }
    let port = if port == 0 { allocate_ephemeral_port()? } else { port };
    if SOCKETS.lock().iter().flatten().any(|other| {
        other.local_port == port
            && other.state != SocketState::Created
            && other.socket_type == descriptor.socket_type
            && (other.local_address == address
                || other.local_address == Ipv4Address([0; 4])
                || address == Ipv4Address([0; 4]))
            && !(other.reuse_address && descriptor.reuse_address)
    }) {
        return Err(Error::Exist);
    }
    if descriptor.socket_type == SocketType::Datagram {
        let handle = descriptor.udp_handle.ok_or(Error::BadFd)?;
        active_context(|context| context.udp_bind(handle, address, port))??;
        descriptor.local_address = address;
        descriptor.local_port = port;
        descriptor.state = SocketState::Bound;
        set_socket_descriptor(index, descriptor);
        return Ok(());
    }
    descriptor.local_address = address;
    descriptor.local_port = port;
    descriptor.state = SocketState::Bound;
    set_socket_descriptor(index, descriptor);
    Ok(())
}

pub(crate) fn listen(file_descriptor: i32, backlog: i32) -> Result<(), Error> {
    let index = socket_index(file_descriptor)?;
    let mut descriptor = socket_descriptor(index)?;
    if descriptor.socket_type != SocketType::Stream {
        return Err(Error::Prototype);
    }
    if !matches!(descriptor.state, SocketState::Created | SocketState::Bound) {
        return Err(Error::Inval);
    }
    if descriptor.local_port == 0 {
        descriptor.local_port = allocate_ephemeral_port()?;
    }
    let local_address = descriptor.local_address;
    let handle = active_context(|context| {
        context.tcp_listen(local_address, descriptor.local_port, backlog.max(1) as usize)
    })??;
    descriptor.handle = Some(handle);
    descriptor.state = SocketState::Listening;
    set_socket_descriptor(index, descriptor);
    Ok(())
}

pub(crate) fn accept(file_descriptor: i32) -> Result<(i32, Ipv4Address, u16), Error> {
    let listener_index = socket_index(file_descriptor)?;
    let listener = socket_descriptor(listener_index)?;
    if listener.socket_type != SocketType::Stream || listener.state != SocketState::Listening {
        return Err(Error::Inval);
    }
    let handle = listener.handle.ok_or(Error::BadFd)?;
    let started = now_ns();
    for _ in 0..IO_POLL_LIMIT {
        let accepted = active_context(|context| {
            context.poll_network();
            context.tcp_accept(handle)
        })??;
        if let Some(accepted) = accepted {
            let (remote_address, remote_port) = active_context(|context| {
                context.tcp_peer_endpoint(accepted)
            })?
            .ok_or(Error::Proto)?;
            let (local_address, local_port) = active_context(|context| {
                context.tcp_local_endpoint(accepted)
            })?
            .ok_or(Error::Proto)?;
            let descriptor = SocketDescriptor {
                socket_type: SocketType::Stream,
                state: SocketState::Connected,
                local_address,
                local_port,
                remote_address,
                remote_port,
                handle: Some(accepted),
                udp_handle: None,
                nonblocking: listener.nonblocking,
                read_shutdown: false,
                write_shutdown: false,
                reuse_address: false,
                broadcast: false,
            };
            let mut sockets = SOCKETS.lock();
            let Some(index) = sockets.iter().position(Option::is_none) else {
                drop(sockets);
                let _removed = active_context(|context| context.tcp_remove(accepted));
                return Err(Error::Mfile);
            };
            sockets[index] = Some(descriptor);
            return Ok((SOCKET_FD_BASE + index as i32, remote_address, remote_port));
        }
        if listener.nonblocking {
            return Err(Error::Again);
        }
        if now_ns().saturating_sub(started) >= CONNECT_TIMEOUT_NS {
            return Err(Error::TimedOut);
        }
        wait_for_interrupt();
    }
    Err(Error::TimedOut)
}

pub(crate) fn connect(
    file_descriptor: i32,
    remote_address: Ipv4Address,
    remote_port: u16,
) -> Result<(), Error> {
    let index = socket_index(file_descriptor)?;
    let mut descriptor = socket_descriptor(index)?;
    if descriptor.socket_type == SocketType::Datagram {
        if remote_port == 0 {
            return Err(Error::Inval);
        }
        if descriptor.local_port == 0 {
            let local_address = active_context(|context| context.local_ipv4())?
                .ok_or(Error::NoDev)?;
            descriptor.local_port = allocate_ephemeral_port()?;
            if descriptor.local_address == Ipv4Address([0; 4]) {
                descriptor.local_address = local_address;
            }
            let handle = descriptor.udp_handle.ok_or(Error::BadFd)?;
            active_context(|context| {
                context.udp_bind(handle, descriptor.local_address, descriptor.local_port)
            })??;
        }
        let handle = descriptor.udp_handle.ok_or(Error::BadFd)?;
        active_context(|context| context.udp_connect(handle, remote_address, remote_port))??;
        descriptor.remote_address = remote_address;
        descriptor.remote_port = remote_port;
        descriptor.state = SocketState::Connected;
        set_socket_descriptor(index, descriptor);
        return Ok(());
    }
    if !matches!(descriptor.state, SocketState::Created | SocketState::Bound | SocketState::Connecting)
        || remote_port == 0
    {
        return Err(Error::Inval);
    }
    if descriptor.state != SocketState::Connecting {
        if descriptor.local_port == 0 {
            descriptor.local_port = allocate_ephemeral_port()?;
        }
        if descriptor.local_address == Ipv4Address([0; 4]) {
            descriptor.local_address = active_context(|context| context.local_ipv4())?
                .ok_or(Error::NoDev)?;
        }
        let handle = active_context(|context| {
            context.tcp_connect(
                descriptor.local_address,
                remote_address,
                remote_port,
                descriptor.local_port,
            )
        })??;
        descriptor.handle = Some(handle);
        descriptor.remote_address = remote_address;
        descriptor.remote_port = remote_port;
        descriptor.state = SocketState::Connecting;
        set_socket_descriptor(index, descriptor);
    }
    if descriptor.nonblocking {
        return Err(Error::InProgress);
    }
    let handle = descriptor.handle.ok_or(Error::BadFd)?;
    let started = now_ns();
    for _ in 0..IO_POLL_LIMIT {
        let state = active_context(|context| {
            context.poll_network();
            context.tcp_state(handle)
        })?;
        match state {
            Some(TcpState::Established) => {
                descriptor.state = SocketState::Connected;
                set_socket_descriptor(index, descriptor);
                return Ok(());
            }
            Some(TcpState::Closed) | None => {
                let _removed = active_context(|context| context.tcp_remove(handle));
                descriptor.handle = None;
                descriptor.state = SocketState::Created;
                set_socket_descriptor(index, descriptor);
                return Err(Error::Io);
            }
            _ => {}
        }
        if now_ns().saturating_sub(started) >= CONNECT_TIMEOUT_NS {
            let _removed = active_context(|context| context.tcp_remove(handle));
            descriptor.handle = None;
            descriptor.state = SocketState::Created;
            set_socket_descriptor(index, descriptor);
            return Err(Error::TimedOut);
        }
        wait_for_interrupt();
    }
    Err(Error::TimedOut)
}

pub(crate) fn send(file_descriptor: i32, bytes: &[u8]) -> Result<usize, Error> {
    let index = socket_index(file_descriptor)?;
    let descriptor = refresh_connection(index, socket_descriptor(index)?)?;
    if descriptor.write_shutdown {
        return Err(Error::Pipe);
    }
    if descriptor.state != SocketState::Connected {
        return Err(if descriptor.state == SocketState::Connecting {
            Error::Again
        } else {
            Error::NotConn
        });
    }
    if descriptor.socket_type == SocketType::Datagram {
        return send_to(
            file_descriptor,
            Some((descriptor.remote_address, descriptor.remote_port)),
            bytes,
        );
    }
    let handle = descriptor.handle.ok_or(Error::BadFd)?;
    let count = active_context(|context| context.tcp_send(handle, bytes))??;
    if count == 0 && !bytes.is_empty() && descriptor.nonblocking {
        Err(Error::Again)
    } else {
        Ok(count)
    }
}

pub(crate) fn send_to(
    file_descriptor: i32,
    destination: Option<(Ipv4Address, u16)>,
    bytes: &[u8],
) -> Result<usize, Error> {
    let index = socket_index(file_descriptor)?;
    let mut descriptor = socket_descriptor(index)?;
    if descriptor.socket_type == SocketType::Stream {
        if let Some((address, port)) = destination {
            if descriptor.remote_address != address || descriptor.remote_port != port {
                return Err(Error::Inval);
            }
        }
        return send(file_descriptor, bytes);
    }
    if descriptor.write_shutdown {
        return Err(Error::Pipe);
    }
    let (address, port) = destination.unwrap_or((descriptor.remote_address, descriptor.remote_port));
    if port == 0 {
        return Err(Error::DestAddrReq);
    }
    let is_broadcast = with_active_tcp_context(|context| {
        context.is_ipv4_broadcast_address(address)
    })
    .ok_or(Error::NoDev)?;
    if is_broadcast && !descriptor.broadcast {
        return Err(Error::Acces);
    }
    descriptor = ensure_udp_binding(index, descriptor)?;
    let handle = descriptor.udp_handle.ok_or(Error::BadFd)?;
    active_context(|context| context.udp_send(handle, address, port, bytes))?
}

pub(crate) fn receive(file_descriptor: i32, output: &mut [u8]) -> Result<usize, Error> {
    receive_from(file_descriptor, output).map(|result| result.0)
}

pub(crate) fn receive_from(
    file_descriptor: i32,
    output: &mut [u8],
) -> Result<(usize, Option<(Ipv4Address, u16)>), Error> {
    let index = socket_index(file_descriptor)?;
    let descriptor = socket_descriptor(index)?;
    if output.is_empty() {
        return Ok((0, None));
    }
    let mut descriptor = refresh_connection(index, descriptor)?;
    if descriptor.read_shutdown {
        return Ok((0, None));
    }
    if descriptor.socket_type == SocketType::Datagram {
        descriptor = ensure_udp_binding(index, descriptor)?;
        let handle = descriptor.udp_handle.ok_or(Error::BadFd)?;
        let started = now_ns();
        for _ in 0..IO_POLL_LIMIT {
            let datagram = active_context(|context| {
                context.poll_network();
                context.udp_receive(handle, output)
            })??;
            if let Some((length, address, port)) = datagram {
                return Ok((length, Some((address, port))));
            }
            if descriptor.nonblocking {
                return Err(Error::Again);
            }
            if now_ns().saturating_sub(started) >= CONNECT_TIMEOUT_NS {
                return Err(Error::TimedOut);
            }
            wait_for_interrupt();
        }
        return Err(Error::TimedOut);
    }
    if descriptor.state != SocketState::Connected {
        return Err(if descriptor.state == SocketState::Connecting {
            Error::Again
        } else {
            Error::NotConn
        });
    }
    let handle = descriptor.handle.ok_or(Error::BadFd)?;
    let started = now_ns();
    for _ in 0..IO_POLL_LIMIT {
        let result = active_context(|context| {
            let count = context.tcp_receive(handle, output)?;
            if count != 0 {
                return Ok((count, None));
            }
            context.poll_network();
            Ok((0, context.tcp_state(handle)))
        })??;
        if result.0 != 0 {
            return Ok((result.0, None));
        }
        if matches!(
            result.1,
            Some(TcpState::CloseWait | TcpState::Closing | TcpState::TimeWait | TcpState::Closed)
        ) {
            return Ok((0, None));
        }
        if descriptor.nonblocking {
            return Err(Error::Again);
        }
        if now_ns().saturating_sub(started) >= CONNECT_TIMEOUT_NS {
            return Err(Error::TimedOut);
        }
        wait_for_interrupt();
    }
    Err(Error::TimedOut)
}

fn ensure_udp_binding(
    index: usize,
    mut descriptor: SocketDescriptor,
) -> Result<SocketDescriptor, Error> {
    if descriptor.socket_type != SocketType::Datagram {
        return Err(Error::Prototype);
    }
    if descriptor.local_port == 0 {
        descriptor.local_port = allocate_ephemeral_port()?;
        if descriptor.local_address == Ipv4Address([0; 4]) {
            descriptor.local_address = active_context(|context| context.local_ipv4())?
                .ok_or(Error::NoDev)?;
        }
        let handle = descriptor.udp_handle.ok_or(Error::BadFd)?;
        active_context(|context| {
            context.udp_bind(handle, descriptor.local_address, descriptor.local_port)
        })??;
        if descriptor.state == SocketState::Created {
            descriptor.state = SocketState::Bound;
        }
        set_socket_descriptor(index, descriptor);
    }
    Ok(descriptor)
}

pub(crate) fn close(file_descriptor: i32) -> Result<(), Error> {
    let index = socket_index(file_descriptor)?;
    let descriptor = socket_descriptor(index)?;
    match descriptor.socket_type {
        SocketType::Stream => {
            if let Some(handle) = descriptor.handle {
                let _ = active_context(|context| {
                    let _closed = context.tcp_close(handle);
                    context.poll_network();
                });
            }
        }
        SocketType::Datagram => {
            if let Some(handle) = descriptor.udp_handle {
                let _ = active_context(|context| context.udp_close(handle));
            }
        }
    }
    SOCKETS.lock()[index] = None;
    Ok(())
}

pub(crate) fn shutdown(file_descriptor: i32, how: c_int) -> Result<(), Error> {
    let index = socket_index(file_descriptor)?;
    let mut descriptor = socket_descriptor(index)?;
    if descriptor.state != SocketState::Connected {
        return Err(Error::NotConn);
    }
    match how {
        0 => descriptor.read_shutdown = true,
        1 => descriptor.write_shutdown = true,
        2 => {
            descriptor.read_shutdown = true;
            descriptor.write_shutdown = true;
        }
        _ => return Err(Error::Inval),
    }
    set_socket_descriptor(index, descriptor);
    if how != 0 && descriptor.socket_type == SocketType::Stream {
        let handle = descriptor.handle.ok_or(Error::BadFd)?;
        active_context(|context| {
            context.tcp_shutdown_write(handle)?;
            context.poll_network();
            Ok::<(), Error>(())
        })??;
    }
    Ok(())
}

pub(crate) fn nonblocking(file_descriptor: i32) -> Result<bool, Error> {
    Ok(socket_descriptor(socket_index(file_descriptor)?)?.nonblocking)
}

pub(crate) fn get_status_flags(file_descriptor: i32) -> Result<i32, Error> {
    let nonblocking = if is_socket_fd(file_descriptor) {
        nonblocking(file_descriptor)?
    } else {
        packet_nonblocking(file_descriptor)?
    };
    Ok(2 | if nonblocking { O_NONBLOCK } else { 0 })
}

pub(crate) fn set_status_flags(file_descriptor: i32, flags: i32) -> Result<(), Error> {
    if is_socket_fd(file_descriptor) {
        set_nonblocking(file_descriptor, flags & O_NONBLOCK != 0)
    } else {
        set_packet_nonblocking(file_descriptor, flags & O_NONBLOCK != 0)
    }
}

pub(crate) fn set_socket_option(
    file_descriptor: i32,
    level: i32,
    option: i32,
    value: i32,
) -> Result<(), Error> {
    if level != SOL_SOCKET {
        return Err(Error::NoProtoOpt);
    }
    if value != 0 && value != 1 {
        return Err(Error::Inval);
    }
    let index = socket_index(file_descriptor)?;
    let mut descriptor = socket_descriptor(index)?;
    match option {
        SO_REUSEADDR => descriptor.reuse_address = value != 0,
        SO_BROADCAST => descriptor.broadcast = value != 0,
        _ => return Err(Error::NoProtoOpt),
    }
    set_socket_descriptor(index, descriptor);
    Ok(())
}

pub(crate) fn get_socket_option(
    file_descriptor: i32,
    level: i32,
    option: i32,
) -> Result<i32, Error> {
    if level != SOL_SOCKET {
        return Err(Error::NoProtoOpt);
    }
    let descriptor = socket_descriptor(socket_index(file_descriptor)?)?;
    match option {
        SO_REUSEADDR => Ok(descriptor.reuse_address as i32),
        SO_BROADCAST => Ok(descriptor.broadcast as i32),
        SO_TYPE => Ok(match descriptor.socket_type {
            SocketType::Stream => SOCK_STREAM,
            SocketType::Datagram => SOCK_DGRAM,
        }),
        SO_DOMAIN => Ok(AF_INET),
        SO_PROTOCOL => Ok(match descriptor.socket_type {
            SocketType::Stream => IPPROTO_TCP,
            SocketType::Datagram => IPPROTO_UDP,
        }),
        SO_ACCEPTCONN => Ok((descriptor.state == SocketState::Listening) as i32),
        SO_ERROR => Ok(if descriptor.state == SocketState::Failed { 5 } else { 0 }),
        SO_SNDBUF | SO_RCVBUF => Ok(SOCKET_BUFFER_SIZE),
        _ => Err(Error::NoProtoOpt),
    }
}

pub(crate) fn endpoint(
    file_descriptor: i32,
    peer: bool,
) -> Result<(Ipv4Address, u16), Error> {
    let index = socket_index(file_descriptor)?;
    let descriptor = socket_descriptor(index)?;
    if descriptor.socket_type == SocketType::Datagram {
        if peer {
            return (descriptor.remote_port != 0)
                .then_some((descriptor.remote_address, descriptor.remote_port))
                .ok_or(Error::NotConn);
        }
        return Ok((descriptor.local_address, descriptor.local_port));
    }
    let descriptor = refresh_connection(index, descriptor)?;
    if peer {
        if descriptor.state != SocketState::Connected {
            return Err(Error::NotConn);
        }
        Ok((descriptor.remote_address, descriptor.remote_port))
    } else {
        Ok((descriptor.local_address, descriptor.local_port))
    }
}

pub(crate) fn readiness(file_descriptor: i32, events: i16) -> Result<i16, Error> {
    let index = socket_index(file_descriptor)?;
    let descriptor = refresh_connection(index, socket_descriptor(index)?)?;
    if descriptor.socket_type == SocketType::Datagram {
        let handle = descriptor.udp_handle.ok_or(Error::BadFd)?;
        let readable = active_context(|context| {
            context.poll_network();
            context.udp_readable(handle)
        })?;
        let mut flags = 0;
        if events & 0x001 != 0 && readable {
            flags |= 0x001;
        }
        if events & 0x004 != 0 {
            flags |= 0x004;
        }
        return Ok(flags);
    }
    let Some(handle) = descriptor.handle else {
        return Ok(0);
    };
    let readiness = active_context(|context| {
        context.poll_network();
        (
            context.tcp_readable(handle),
            context.tcp_writable(handle),
            context.tcp_state(handle),
        )
    })?
    .ok_or(Error::NoDev)?;
    let mut flags = 0;
    if events & 0x001 != 0 && readiness.0 {
        flags |= 0x001;
    }
    if events & 0x004 != 0 && readiness.1 {
        flags |= 0x004;
    }
    if matches!(readiness.2, Some(TcpState::Closing | TcpState::TimeWait | TcpState::Closed))
        || descriptor.state == SocketState::Failed
    {
        flags |= 0x008 | 0x010;
    }
    Ok(flags)
}

fn active_context<R>(operation: impl FnOnce(&mut TcpContext) -> R) -> Result<R, Error> {
    with_active_tcp_context(operation).ok_or(Error::NoDev)
}

fn socket_index(file_descriptor: i32) -> Result<usize, Error> {
    let index = file_descriptor
        .checked_sub(SOCKET_FD_BASE)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(Error::BadFd)?;
    (index < SOCKET_COUNT).then_some(index).ok_or(Error::BadFd)
}

fn packet_index(file_descriptor: i32) -> Result<usize, Error> {
    let index = file_descriptor
        .checked_sub(PACKET_FD_BASE)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(Error::BadFd)?;
    (index < PACKET_DEVICE_COUNT).then_some(index).ok_or(Error::BadFd)
}

fn packet_descriptor(file_descriptor: i32) -> Result<PacketDescriptor, Error> {
    PACKET_DEVICES.lock()[packet_index(file_descriptor)?]
        .clone()
        .ok_or(Error::BadFd)
}

fn default_interface_name(mode: TunTapMode, index: usize) -> [u8; 16] {
    let mut name = [0; 16];
    let prefix = match mode {
        TunTapMode::Tun => *b"tun",
        TunTapMode::Tap => *b"tap",
    };
    name[..3].copy_from_slice(&prefix);
    name[3] = if index < 10 {
        b'0' + index as u8
    } else {
        b'a' + (index as u8 - 10)
    };
    name
}

fn socket_descriptor(index: usize) -> Result<SocketDescriptor, Error> {
    SOCKETS.lock()[index].ok_or(Error::BadFd)
}

fn set_socket_descriptor(index: usize, descriptor: SocketDescriptor) {
    SOCKETS.lock()[index] = Some(descriptor);
}

fn refresh_connection(
    index: usize,
    mut descriptor: SocketDescriptor,
) -> Result<SocketDescriptor, Error> {
    if descriptor.state != SocketState::Connecting {
        return Ok(descriptor);
    }
    let Some(handle) = descriptor.handle else {
        descriptor.state = SocketState::Failed;
        set_socket_descriptor(index, descriptor);
        return Ok(descriptor);
    };
    let state = active_context(|context| {
        context.poll_network();
        context.tcp_state(handle)
    })?;
    match state {
        Some(TcpState::Established) => descriptor.state = SocketState::Connected,
        Some(TcpState::Closed) | None => descriptor.state = SocketState::Failed,
        _ => {}
    }
    set_socket_descriptor(index, descriptor);
    Ok(descriptor)
}

fn allocate_ephemeral_port() -> Result<u16, Error> {
    for _ in 0..usize::from(EPHEMERAL_PORT_COUNT) {
        let offset = NEXT_EPHEMERAL_PORT.fetch_add(1, Ordering::Relaxed) % EPHEMERAL_PORT_COUNT;
        let port = FIRST_EPHEMERAL_PORT + offset;
        let occupied = SOCKETS
            .lock()
            .iter()
            .flatten()
            .any(|descriptor| descriptor.local_port == port);
        if !occupied {
            return Ok(port);
        }
    }
    Err(Error::NoBufs)
}

fn now_ns() -> u64 {
    crate::kernel::arch::TimerArchitecture::now_ns()
        .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns)
}

fn wait_for_interrupt() {
    crate::kernel::runtime::runtime_ffi::kz_host_wait_for_interrupt();
}