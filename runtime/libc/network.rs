/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::ffi::{c_int, c_void};
use core::ptr;
use core::sync::atomic::{AtomicU16, Ordering};

use crate::kernel::arch::traits::TimerArchitecture as _;
use crate::kernel::error::Error;
use crate::kernel::net::tcp::connection::{TcpSocketHandle, TcpState};
use crate::kernel::net::udp::UdpSocketHandle;
use crate::kernel::net::wire::Ipv4Address;
use crate::kernel::runtime::runtime_ffi::{
    with_active_tcp_context, TcpContext,
};
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::vfs::plugin::tun::{TunDevice, TunTapMode};

pub(crate) const SOCKET_FD_BASE: i32 = 128;
const SOCKET_COUNT: usize = 32;
pub(crate) const LOCAL_SOCKET_FD_BASE: i32 = 320;
const LOCAL_SOCKET_COUNT: usize = 32;
const LOCAL_SOCKET_BUFFER_SIZE: usize = 65_536;
const AF_UNIX: i32 = 1;
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
const SO_OOBINLINE: i32 = 10;
const SO_PASSCRED: i32 = 16;
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

pub(crate) fn ioctl_requires_admin(request: usize) -> bool {
    matches!(
        request,
        TUNSETIFF | SIOCSIFFLAGS | SIOCSIFADDR | SIOCSIFNETMASK
    )
}
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
static SOCKETS: IrqSpinLock<[Option<SocketFd>; SOCKET_COUNT]> =
    IrqSpinLock::new([const { None }; SOCKET_COUNT]);
static LOCAL_SOCKETS: IrqSpinLock<[Option<LocalSocketFd>; LOCAL_SOCKET_COUNT]> =
    IrqSpinLock::new([const { None }; LOCAL_SOCKET_COUNT]);
static LOCAL_SOCKET_PATHS: IrqSpinLock<Vec<(Vec<u8>, Weak<IrqSpinLock<LocalOpenDescription>>)>> =
    IrqSpinLock::new(Vec::new());
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
    descriptor_references: usize,
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
struct SocketFd {
    owner_process_id: i32,
    description: Arc<IrqSpinLock<SocketDescriptor>>,
    close_on_exec: bool,
}

#[derive(Clone)]
struct PacketDescriptor {
    owner_process_id: i32,
    device: Arc<TunDevice>,
    mode: Option<TunTapMode>,
    no_packet_info: bool,
    nonblocking: bool,
    close_on_exec: bool,
    name: [u8; 16],
}

struct LocalPair {
    queues: [VecDeque<u8>; 2],
    bytes_written: [u64; 2],
    bytes_read: [u64; 2],
    readers: [usize; 2],
    writers: [usize; 2],
    read_shutdown: [bool; 2],
    write_shutdown: [bool; 2],
    control_messages: [VecDeque<QueuedControlMessage>; 2],
    out_of_band: [Option<(u8, u64)>; 2],
    pass_credentials: [bool; 2],
    out_of_band_inline: [bool; 2],
}

#[derive(Clone)]
pub(crate) struct LocalControlMessage {
    pub(crate) rights: Vec<i32>,
    pub(crate) credentials: Option<LocalCredentials>,
}

pub(crate) struct LocalReceiveResult {
    pub(crate) length: usize,
    pub(crate) control_messages: Vec<LocalControlMessage>,
}

#[derive(Clone, Copy)]
pub(crate) struct LocalCredentials {
    pub(crate) process_id: i32,
    pub(crate) user_id: u32,
    pub(crate) group_id: u32,
}

struct QueuedControlMessage {
    position: u64,
    message: LocalControlMessage,
}

struct LocalListener {
    path: Vec<u8>,
    backlog: usize,
    closed: bool,
    pending: VecDeque<PendingLocalConnection>,
}

struct PendingLocalConnection {
    pair: Arc<IrqSpinLock<LocalPair>>,
    peer_path: Option<Vec<u8>>,
}

struct LocalOpenDescription {
    pair: Option<Arc<IrqSpinLock<LocalPair>>>,
    side: usize,
    descriptor_references: usize,
    nonblocking: bool,
    connected: bool,
    closed: bool,
    listener: Option<Arc<IrqSpinLock<LocalListener>>>,
    local_path: Option<Vec<u8>>,
    peer_path: Option<Vec<u8>>,
    pass_credentials: bool,
    out_of_band_inline: bool,
}

#[derive(Clone)]
struct LocalSocketFd {
    owner_process_id: i32,
    description: Arc<IrqSpinLock<LocalOpenDescription>>,
    close_on_exec: bool,
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
    if let Some(alias) = crate::kernel::runtime::runtime_libc::fd_alias::get(file_descriptor) {
        return alias.kind == crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind::Socket;
    }
    socket_index(file_descriptor).is_ok() || local_socket_index(file_descriptor).is_ok()
}

pub(crate) fn has_socket_fd(file_descriptor: i32) -> bool {
    ThreadManager::current_process().ok().is_some_and(|process| {
        has_socket_fd_for_process(process.pid(), file_descriptor)
    })
}

pub(crate) fn has_socket_fd_for_process(process_id: i32, file_descriptor: i32) -> bool {
    if let Some(alias) = crate::kernel::runtime::runtime_libc::fd_alias::get_for_process(
        process_id,
        file_descriptor,
    ) {
        if alias.kind != crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind::Socket {
            return false;
        }
        return local_socket_index_direct_for_process(process_id, alias.backing).is_ok()
            || socket_index_direct_for_process(process_id, alias.backing).is_ok();
    }
    if crate::kernel::runtime::runtime_libc::fd_alias::is_backing_for_process(
        process_id,
        file_descriptor,
    ) {
        return false;
    }
    if let Ok(index) = local_socket_index_for_process(process_id, file_descriptor) {
        return LOCAL_SOCKETS.lock()[index]
            .as_ref()
            .is_some_and(|descriptor| descriptor.owner_process_id == process_id);
    }
    socket_index_for_process(process_id, file_descriptor)
        .ok()
        .is_some_and(|index| {
            SOCKETS.lock()[index]
                .as_ref()
                .is_some_and(|descriptor| descriptor.owner_process_id == process_id)
        })
}

pub(crate) fn socket_backing_is_open_for_process(
    process_id: i32,
    file_descriptor: i32,
) -> bool {
    local_socket_index_direct_for_process(process_id, file_descriptor).is_ok()
        || socket_index_direct_for_process(process_id, file_descriptor).is_ok()
}

pub(crate) fn is_local_socket_fd(file_descriptor: i32) -> bool {
    ThreadManager::current_process().ok().is_some_and(|process| {
        is_local_socket_fd_for_process(process.pid(), file_descriptor)
    })
}

fn is_local_socket_fd_for_process(process_id: i32, file_descriptor: i32) -> bool {
    if let Some(alias) = crate::kernel::runtime::runtime_libc::fd_alias::get_for_process(
        process_id,
        file_descriptor,
    ) {
        return alias.kind == crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind::Socket
            && local_socket_index_direct_for_process(process_id, alias.backing).is_ok();
    }
    if crate::kernel::runtime::runtime_libc::fd_alias::is_backing_for_process(
        process_id,
        file_descriptor,
    ) {
        return false;
    }
    local_socket_index_for_process(process_id, file_descriptor).is_ok()
}

pub(crate) fn same_socket_table(source: i32, target: i32) -> bool {
    if is_local_socket_fd(source) {
        is_local_socket_fd(target)
    } else {
        socket_index(source).is_ok() && socket_index(target).is_ok()
    }
}

fn local_socket_index(file_descriptor: i32) -> Result<usize, Error> {
    let process_id = ThreadManager::current_process()?.pid();
    local_socket_index_for_process(process_id, file_descriptor)
}

fn local_socket_index_for_process(process_id: i32, file_descriptor: i32) -> Result<usize, Error> {
    let file_descriptor = crate::kernel::runtime::runtime_libc::fd_alias::resolve_for_process(
        process_id,
        file_descriptor,
        crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind::Socket,
    )?;
    let index = file_descriptor
        .checked_sub(LOCAL_SOCKET_FD_BASE)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(Error::BadFd)?;
    if index >= LOCAL_SOCKET_COUNT
        || !LOCAL_SOCKETS.lock()[index]
            .as_ref()
            .is_some_and(|descriptor| descriptor.owner_process_id == process_id)
    {
        return Err(Error::BadFd);
    }
    Ok(index)
}

fn local_socket_index_direct_for_process(
    process_id: i32,
    file_descriptor: i32,
) -> Result<usize, Error> {
    let index = local_socket_slot_index(file_descriptor)?;
    if !LOCAL_SOCKETS.lock()[index]
        .as_ref()
        .is_some_and(|descriptor| descriptor.owner_process_id == process_id)
    {
        return Err(Error::BadFd);
    }
    Ok(index)
}

fn local_socket_slot_index(file_descriptor: i32) -> Result<usize, Error> {
    let index = file_descriptor
        .checked_sub(LOCAL_SOCKET_FD_BASE)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(Error::BadFd)?;
    (index < LOCAL_SOCKET_COUNT).then_some(index).ok_or(Error::BadFd)
}

fn local_socket_descriptor(file_descriptor: i32) -> Result<LocalSocketFd, Error> {
    let index = local_socket_index(file_descriptor)?;
    LOCAL_SOCKETS.lock()[index].clone().ok_or(Error::BadFd)
}

fn local_socket_descriptor_for_process(
    process_id: i32,
    file_descriptor: i32,
) -> Result<LocalSocketFd, Error> {
    let index = local_socket_index_for_process(process_id, file_descriptor)?;
    LOCAL_SOCKETS.lock()[index].clone().ok_or(Error::BadFd)
}

fn local_socket_description(
    file_descriptor: i32,
) -> Result<Arc<IrqSpinLock<LocalOpenDescription>>, Error> {
    Ok(local_socket_descriptor(file_descriptor)?.description)
}

fn empty_local_pair() -> Result<Arc<IrqSpinLock<LocalPair>>, Error> {
    let mut queues = [VecDeque::new(), VecDeque::new()];
    for queue in &mut queues {
        queue
            .try_reserve_exact(LOCAL_SOCKET_BUFFER_SIZE)
            .map_err(|_| Error::NoMem)?;
    }
    let mut control_messages = [VecDeque::new(), VecDeque::new()];
    for messages in &mut control_messages {
        messages.try_reserve(16).map_err(|_| Error::NoMem)?;
    }
    Ok(Arc::new(IrqSpinLock::new(LocalPair {
        queues,
        bytes_written: [0; 2],
        bytes_read: [0; 2],
        readers: [1; 2],
        writers: [1; 2],
        read_shutdown: [false; 2],
        write_shutdown: [false; 2],
        control_messages,
        out_of_band: [None, None],
        pass_credentials: [false; 2],
        out_of_band_inline: [false; 2],
    })))
}
pub(crate) fn validate_local_socket(file_descriptor: i32) -> Result<(), Error> {
    let description = local_socket_description(file_descriptor)?;
    if description.lock().closed {
        Err(Error::BadFd)
    } else {
        Ok(())
    }
}

pub(crate) fn local_socket_names(file_descriptor: i32, peer: bool) -> Result<Vec<u8>, Error> {
    let description = local_socket_description(file_descriptor)?;
    let description = description.lock();
    if description.closed {
        return Err(Error::BadFd);
    }
    if peer && !description.connected {
        return Err(Error::NotConn);
    }
    let path = if peer {
        description.peer_path.as_deref()
    } else {
        description.local_path.as_deref()
    };
    let mut result = Vec::new();
    if let Some(path) = path {
        result.try_reserve_exact(path.len()).map_err(|_| Error::NoMem)?;
        result.extend_from_slice(path);
    }
    Ok(result)
}

pub(crate) fn bind_local_socket(file_descriptor: i32, path: &[u8]) -> Result<(), Error> {
    if path.is_empty() || path.contains(&0) {
        return Err(Error::Inval);
    }
    let description = local_socket_description(file_descriptor)?;
    {
        let mut state = description.lock();
        if state.closed || state.local_path.is_some() || state.connected || state.listener.is_some() {
            return Err(Error::Inval);
        }
        let mut owned_path = Vec::new();
        owned_path.try_reserve_exact(path.len()).map_err(|_| Error::NoMem)?;
        owned_path.extend_from_slice(path);
        state.local_path = Some(owned_path);
    }
    let mut paths = LOCAL_SOCKET_PATHS.lock();
    paths.retain(|(_, socket)| socket.strong_count() != 0);
    if paths.iter().any(|(registered, _)| registered.as_slice() == path) {
        description.lock().local_path = None;
        return Err(Error::AddrInUse);
    }
    let mut owned_path = Vec::new();
    if owned_path.try_reserve_exact(path.len()).is_err() {
        description.lock().local_path = None;
        return Err(Error::NoMem);
    }
    owned_path.extend_from_slice(path);
    if paths.try_reserve(1).is_err() {
        description.lock().local_path = None;
        return Err(Error::NoMem);
    }
    paths.push((owned_path, Arc::downgrade(&description)));
    Ok(())
}

pub(crate) fn unlink_local_socket_path(path: &[u8]) {
    LOCAL_SOCKET_PATHS
        .lock()
        .retain(|(registered, _)| registered.as_slice() != path);
}

pub(crate) fn listen_local_socket(file_descriptor: i32, backlog: i32) -> Result<(), Error> {
    let description = local_socket_description(file_descriptor)?;
    let mut state = description.lock();
    if state.closed || state.connected || state.listener.is_some() {
        return Err(Error::Inval);
    }
    let path = state.local_path.as_ref().ok_or(Error::Inval)?;
    let mut pending = VecDeque::new();
    pending.try_reserve(backlog.max(1) as usize).map_err(|_| Error::NoMem)?;
    state.listener = Some(Arc::new(IrqSpinLock::new(LocalListener {
        path: path.clone(),
        backlog: backlog.max(1) as usize,
        closed: false,
        pending,
    })));
    Ok(())
}

pub(crate) fn connect_local_socket(file_descriptor: i32, path: &[u8]) -> Result<(), Error> {
    let listener_description = {
        let paths = LOCAL_SOCKET_PATHS.lock();
        paths
            .iter()
            .find(|(registered, _)| registered.as_slice() == path)
            .and_then(|(_, socket)| socket.upgrade())
            .ok_or(Error::ConnRefused)?
    };
    let (listener, listener_path, listener_pass_credentials, listener_inline) = {
        let listener_state = listener_description.lock();
        if listener_state.closed {
            return Err(Error::ConnRefused);
        }
        (
            listener_state.listener.as_ref().map(Arc::clone).ok_or(Error::ConnRefused)?,
            listener_state.local_path.clone().ok_or(Error::ConnRefused)?,
            listener_state.pass_credentials,
            listener_state.out_of_band_inline,
        )
    };
    let pair = empty_local_pair()?;
    let client_description = local_socket_description(file_descriptor)?;
    let (client_path, client_nonblocking, client_pass_credentials, client_inline) = {
        let client = client_description.lock();
        if client.closed || client.connected || client.listener.is_some() {
            return Err(Error::IsConn);
        }
        (
            client.local_path.clone(),
            client.nonblocking,
            client.pass_credentials,
            client.out_of_band_inline,
        )
    };
    {
        let mut pair = pair.lock();
        pair.pass_credentials[0] = client_pass_credentials;
        pair.pass_credentials[1] = listener_pass_credentials;
        pair.out_of_band_inline[0] = client_inline;
        pair.out_of_band_inline[1] = listener_inline;
    }
    loop {
        ThreadManager::cancel_current_if_requested();
        let mut client = client_description.lock();
        if client.closed || client.connected || client.listener.is_some() {
            return Err(Error::IsConn);
        }
        let mut listener = listener.lock();
        if listener.closed {
            return Err(Error::ConnRefused);
        }
        if listener.pending.len() >= listener.backlog {
            drop(listener);
            drop(client);
            if client_nonblocking {
                return Err(Error::Again);
            }
            if wait_for_interrupt() && !restart_interrupted_io() {
                return Err(Error::Intr);
            }
            continue;
        }
        listener.pending.try_reserve(1).map_err(|_| Error::NoMem)?;
        client.pair = Some(Arc::clone(&pair));
        client.side = 0;
        client.connected = true;
        client.peer_path = Some(listener_path.clone());
        listener.pending.push_back(PendingLocalConnection {
            pair: Arc::clone(&pair),
            peer_path: client_path.clone(),
        });
        return Ok(());
    }
}

fn accept_local_socket(file_descriptor: i32) -> Result<i32, Error> {
    let process_id = ThreadManager::current_process()?.pid();
    let listener_description = local_socket_description(file_descriptor)?;
    let (listener, listener_path, nonblocking) = {
        let state = listener_description.lock();
        (
            state.listener.as_ref().map(Arc::clone).ok_or(Error::Inval)?,
            state.local_path.clone().ok_or(Error::Inval)?,
            state.nonblocking,
        )
    };
    loop {
        ThreadManager::cancel_current_if_requested();
        let mut sockets = LOCAL_SOCKETS.lock();
        let target_index = available_local_socket_index(&sockets, process_id)
            .ok_or(Error::Mfile)?;
        crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(
            LOCAL_SOCKET_FD_BASE + target_index as i32,
        )?;
        let pending = {
            let mut listener = listener.lock();
            if listener.closed {
                return Err(Error::ConnReset);
            }
            listener.pending.pop_front()
        };
        if let Some(pending) = pending {
            let accepted = LocalSocketFd {
                owner_process_id: process_id,
                description: Arc::new(IrqSpinLock::new(LocalOpenDescription {
                    pair: Some(pending.pair),
                    side: 1,
                    descriptor_references: 1,
                    nonblocking,
                    connected: true,
                    closed: false,
                    listener: None,
                    local_path: Some(listener_path.clone()),
                    peer_path: pending.peer_path,
                    pass_credentials: false,
                    out_of_band_inline: false,
                })),
                close_on_exec: false,
            };
            if sockets[target_index].is_some() {
                return Err(Error::Mfile);
            }
            sockets[target_index] = Some(accepted);
            return Ok(LOCAL_SOCKET_FD_BASE + target_index as i32);
        }
        drop(sockets);
        if nonblocking {
            return Err(Error::Again);
        }
        if wait_for_interrupt() && !restart_interrupted_io() {
            return Err(Error::Intr);
        }
    }
}

pub(crate) fn is_packet_fd(file_descriptor: i32) -> bool {
    if let Some(alias) = crate::kernel::runtime::runtime_libc::fd_alias::get(file_descriptor) {
        return alias.kind == crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind::Packet;
    }
    packet_index(file_descriptor).is_ok()
}

pub(crate) fn has_packet_fd(file_descriptor: i32) -> bool {
    ThreadManager::current_process().ok().is_some_and(|process| {
        has_packet_fd_for_process(process.pid(), file_descriptor)
    })
}

pub(crate) fn has_packet_fd_for_process(process_id: i32, file_descriptor: i32) -> bool {
    if let Some(alias) = crate::kernel::runtime::runtime_libc::fd_alias::get_for_process(
        process_id,
        file_descriptor,
    ) {
        return alias.kind == crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind::Packet
            && packet_index_direct_for_process(process_id, alias.backing).is_ok();
    }
    if crate::kernel::runtime::runtime_libc::fd_alias::is_backing_for_process(
        process_id,
        file_descriptor,
    ) {
        return false;
    }
    packet_index_for_process(process_id, file_descriptor)
        .ok()
        .is_some_and(|index| {
            PACKET_DEVICES.lock()[index]
                .as_ref()
                .is_some_and(|descriptor| descriptor.owner_process_id == process_id)
        })
}

pub(crate) fn packet_backing_is_open_for_process(
    process_id: i32,
    file_descriptor: i32,
) -> bool {
    packet_index_direct_for_process(process_id, file_descriptor).is_ok()
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

pub(crate) fn open_tun_device(nonblocking: bool, close_on_exec: bool) -> Result<i32, Error> {
    let process_id = ThreadManager::current_process()?.pid();
    open_tun_device_for_process(nonblocking, close_on_exec, process_id)
}

pub(crate) fn open_tun_device_for_process(
    nonblocking: bool,
    close_on_exec: bool,
    process_id: i32,
) -> Result<i32, Error> {
    let device = TunDevice::new()?;
    let mut devices = PACKET_DEVICES.lock();
    let index = devices
        .iter()
        .enumerate()
        .find_map(|(index, descriptor)| {
            let target = PACKET_FD_BASE + index as i32;
            (descriptor.is_none()
                && !crate::kernel::runtime::runtime_libc::fd_alias::contains_target_for_process(
                    process_id,
                    target,
                ))
            .then_some(index)
        })
        .ok_or(Error::Mfile)?;
    crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(
        PACKET_FD_BASE + index as i32,
    )?;
    devices[index] = Some(PacketDescriptor {
        owner_process_id: process_id,
        device,
        mode: None,
        no_packet_info: false,
        nonblocking,
        close_on_exec,
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
            #[cfg(feature = "virtualization")]
            crate::kernel::vm::device::virtio::net::poll_tap_devices();
        }
    }
    Ok(length)
}

pub(crate) fn pump_packet_network() {
    let _progress = with_active_tcp_context(TcpContext::poll_network);
    #[cfg(feature = "virtualization")]
    crate::kernel::vm::device::virtio::net::poll_tap_devices();
}

pub(crate) fn close_packet_fd(file_descriptor: i32) -> Result<(), Error> {
    let process_id = ThreadManager::current_process()?.pid();
    close_packet_fd_for_process(process_id, file_descriptor)
}

pub(crate) fn close_packet_fd_for_process(
    process_id: i32,
    file_descriptor: i32,
) -> Result<(), Error> {
    if let Some(alias) = crate::kernel::runtime::runtime_libc::fd_alias::get_for_process(
        process_id,
        file_descriptor,
    ) {
        if alias.kind != crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind::Packet {
            return Err(Error::BadFd);
        }
        return close_packet_fd_direct_for_process(process_id, alias.backing);
    }
    if crate::kernel::runtime::runtime_libc::fd_alias::is_backing_for_process(
        process_id,
        file_descriptor,
    ) {
        return Err(Error::BadFd);
    }
    close_packet_fd_direct_for_process(process_id, file_descriptor)
}

pub(crate) fn close_packet_fd_direct_for_process(
    process_id: i32,
    file_descriptor: i32,
) -> Result<(), Error> {
    let index = packet_index_direct_for_process(process_id, file_descriptor)?;
    let descriptor = PACKET_DEVICES.lock()[index].take().ok_or(Error::BadFd)?;
    let still_open = PACKET_DEVICES
        .lock()
        .iter()
        .flatten()
        .any(|other| Arc::ptr_eq(&other.device, &descriptor.device));
    if !still_open {
        descriptor.device.set_up(false);
    }
    Ok(())
}

pub(crate) fn duplicate_packet_fd(
    file_descriptor: i32,
    minimum: i32,
    close_on_exec: bool,
) -> Result<i32, Error> {
    let process_id = ThreadManager::current_process()?.pid();
    duplicate_packet_fd_internal(
        file_descriptor,
        minimum,
        close_on_exec,
        true,
        process_id,
        process_id,
    )
}

pub(crate) fn duplicate_packet_fd_backing(file_descriptor: i32) -> Result<i32, Error> {
    let process_id = ThreadManager::current_process()?.pid();
    duplicate_packet_fd_for_process(file_descriptor, process_id, process_id)
}

pub(crate) fn duplicate_packet_fd_for_process(
    file_descriptor: i32,
    source_process_id: i32,
    target_process_id: i32,
) -> Result<i32, Error> {
    duplicate_packet_fd_internal(
        file_descriptor,
        PACKET_FD_BASE,
        false,
        false,
        source_process_id,
        target_process_id,
    )
}

fn duplicate_packet_fd_internal(
    file_descriptor: i32,
    minimum: i32,
    close_on_exec: bool,
    enforce_limit: bool,
    source_process_id: i32,
    target_process_id: i32,
) -> Result<i32, Error> {
    if minimum < 0 {
        return Err(Error::Inval);
    }
    let source_index = packet_index_for_process(source_process_id, file_descriptor)?;
    let first = usize::try_from(minimum.saturating_sub(PACKET_FD_BASE))
        .unwrap_or(0)
        .min(PACKET_DEVICE_COUNT);
    let mut devices = PACKET_DEVICES.lock();
    let source = devices[source_index].as_ref().cloned().ok_or(Error::BadFd)?;
    let target_index = devices
        .iter()
        .enumerate()
        .skip(first)
        .find_map(|(index, descriptor)| {
            let target = PACKET_FD_BASE + index as i32;
            (descriptor.is_none()
                && !crate::kernel::runtime::runtime_libc::fd_alias::contains_target_for_process(
                    target_process_id,
                    target,
                ))
            .then_some(index)
        })
        .ok_or(Error::Mfile)?;
    if enforce_limit {
        crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(
            PACKET_FD_BASE + target_index as i32,
        )?;
    }
    devices[target_index] = Some(PacketDescriptor {
        owner_process_id: target_process_id,
        close_on_exec,
        ..source
    });
    Ok(PACKET_FD_BASE + target_index as i32)
}

pub(crate) fn duplicate_packet_fd_to(
    file_descriptor: i32,
    target: i32,
    close_on_exec: bool,
) -> Result<i32, Error> {
    let process_id = ThreadManager::current_process()?.pid();
    let source_index = packet_index_for_process(process_id, file_descriptor)?;
    let target_index = packet_slot_index(target)?;
    if file_descriptor == target {
        return if close_on_exec { Err(Error::Inval) } else { Ok(target) };
    }
    crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(target)?;
    let target_owner = PACKET_DEVICES.lock()[target_index]
        .as_ref()
        .map(|existing| existing.owner_process_id);
    if let Some(target_owner) = target_owner {
        if target_owner != process_id {
            return Err(Error::BadFd);
        }
        close_packet_fd_for_process(process_id, target)?;
    }
    let mut devices = PACKET_DEVICES.lock();
    let mut descriptor = devices[source_index].as_ref().cloned().ok_or(Error::BadFd)?;
    descriptor.owner_process_id = process_id;
    descriptor.close_on_exec = close_on_exec;
    devices[target_index] = Some(descriptor);
    Ok(target)
}

pub(crate) fn packet_descriptor_flags(file_descriptor: i32) -> Result<i32, Error> {
    Ok(i32::from(packet_descriptor(file_descriptor)?.close_on_exec))
}

pub(crate) fn set_packet_descriptor_flags(file_descriptor: i32, flags: i32) -> Result<(), Error> {
    if flags & !1 != 0 {
        return Err(Error::Inval);
    }
    let index = packet_index(file_descriptor)?;
    PACKET_DEVICES.lock()[index]
        .as_mut()
        .ok_or(Error::BadFd)?
        .close_on_exec = flags & 1 != 0;
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
    if domain == AF_UNIX {
        return create_local_socket(socket_type, protocol);
    }
    if domain != AF_INET {
        return Err(Error::AfNoSupport);
    }
    let process_id = ThreadManager::current_process()?.pid();
    let nonblocking = socket_type & SOCK_NONBLOCK != 0;
    let close_on_exec = socket_type & SOCK_CLOEXEC != 0;
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
        descriptor_references: 1,
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
        .enumerate()
        .find_map(|(index, descriptor)| {
            let target = SOCKET_FD_BASE + index as i32;
            (descriptor.is_none()
                && !crate::kernel::runtime::runtime_libc::fd_alias::contains_target_for_process(
                    process_id,
                    target,
                ))
            .then_some(index)
        })
        .ok_or(Error::Mfile)?;
    if let Err(error) = crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(
        SOCKET_FD_BASE + index as i32,
    ) {
        drop(sockets);
        if let Some(handle) = udp_handle {
            let _ = active_context(|context| context.udp_close(handle));
        }
        return Err(error);
    }
    sockets[index] = Some(SocketFd {
        owner_process_id: process_id,
        description: Arc::new(IrqSpinLock::new(descriptor)),
        close_on_exec,
    });
    Ok(SOCKET_FD_BASE + index as i32)
}

fn available_local_socket_index(
    sockets: &[Option<LocalSocketFd>; LOCAL_SOCKET_COUNT],
    process_id: i32,
) -> Option<usize> {
    sockets.iter().enumerate().find_map(|(index, socket)| {
        let descriptor = LOCAL_SOCKET_FD_BASE + index as i32;
        (socket.is_none()
            && !crate::kernel::runtime::runtime_libc::fd_alias::contains_target_for_process(
                process_id,
                descriptor,
            ))
        .then_some(index)
    })
}

fn create_local_socket(socket_type: i32, protocol: i32) -> Result<i32, Error> {
    let process_id = ThreadManager::current_process()?.pid();
    let nonblocking = socket_type & SOCK_NONBLOCK != 0;
    let close_on_exec = socket_type & SOCK_CLOEXEC != 0;
    if socket_type & !(SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC | SOCK_CLOFORK) != 0
        || socket_type & SOCK_STREAM == 0
        || protocol != 0
    {
        return Err(Error::SockTypeNoSupport);
    }
    let mut sockets = LOCAL_SOCKETS.lock();
    let index = available_local_socket_index(&sockets, process_id).ok_or(Error::Mfile)?;
    crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(
        LOCAL_SOCKET_FD_BASE + index as i32,
    )?;
    sockets[index] = Some(LocalSocketFd {
        owner_process_id: process_id,
        description: Arc::new(IrqSpinLock::new(LocalOpenDescription {
            pair: None,
            side: 0,
            descriptor_references: 1,
            nonblocking,
            connected: false,
            closed: false,
            listener: None,
            local_path: None,
            peer_path: None,
            pass_credentials: false,
            out_of_band_inline: false,
        })),
        close_on_exec,
    });
    Ok(LOCAL_SOCKET_FD_BASE + index as i32)
}

pub(crate) fn create_local_socket_pair(
    domain: i32,
    socket_type: i32,
    protocol: i32,
) -> Result<(i32, i32), Error> {
    if domain != AF_UNIX {
        return Err(Error::AfNoSupport);
    }
    let process_id = ThreadManager::current_process()?.pid();
    let nonblocking = socket_type & SOCK_NONBLOCK != 0;
    let close_on_exec = socket_type & SOCK_CLOEXEC != 0;
    if socket_type & !(SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC | SOCK_CLOFORK) != 0
        || socket_type & SOCK_STREAM == 0
        || protocol != 0
    {
        return Err(Error::SockTypeNoSupport);
    }
    let pair = empty_local_pair()?;
    let mut sockets = LOCAL_SOCKETS.lock();
    let first = available_local_socket_index(&sockets, process_id).ok_or(Error::Mfile)?;
    let second = sockets
        .iter()
        .enumerate()
        .find_map(|(index, socket)| {
            let descriptor = LOCAL_SOCKET_FD_BASE + index as i32;
            (index != first
                && socket.is_none()
                && !crate::kernel::runtime::runtime_libc::fd_alias::contains_target_for_process(
                    process_id,
                    descriptor,
                ))
            .then_some(index)
        })
        .ok_or(Error::Mfile)?;
    for index in [first, second] {
        crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(
            LOCAL_SOCKET_FD_BASE + index as i32,
        )?;
    }
    for (index, side) in [(first, 0), (second, 1)] {
        sockets[index] = Some(LocalSocketFd {
            owner_process_id: process_id,
            description: Arc::new(IrqSpinLock::new(LocalOpenDescription {
                pair: Some(Arc::clone(&pair)),
                side,
                descriptor_references: 1,
                nonblocking,
                connected: true,
                closed: false,
                listener: None,
                local_path: None,
                peer_path: None,
                pass_credentials: false,
                out_of_band_inline: false,
            })),
            close_on_exec,
        });
    }
    Ok((
        LOCAL_SOCKET_FD_BASE + first as i32,
        LOCAL_SOCKET_FD_BASE + second as i32,
    ))
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
    let _local_address = local_address.ok_or(Error::NoDev)?;
    if address != Ipv4Address([0; 4]) && !is_local_address {
        return Err(Error::Inval);
    }
    let port = if port == 0 { allocate_ephemeral_port()? } else { port };
    if SOCKETS.lock().iter().flatten().any(|other| {
        let other = *other.description.lock();
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
    if is_local_socket_fd(file_descriptor) {
        return listen_local_socket(file_descriptor, backlog);
    }
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
    if is_local_socket_fd(file_descriptor) {
        return accept_local_socket(file_descriptor)
            .map(|descriptor| (descriptor, Ipv4Address([0; 4]), 0));
    }
    let process_id = ThreadManager::current_process()?.pid();
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
                descriptor_references: 1,
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
            let Some(index) = sockets.iter().enumerate().find_map(|(index, descriptor)| {
                let target = SOCKET_FD_BASE + index as i32;
                (descriptor.is_none()
                    && !crate::kernel::runtime::runtime_libc::fd_alias::contains_target_for_process(
                        process_id,
                        target,
                    ))
                .then_some(index)
            }) else {
                drop(sockets);
                let _removed = active_context(|context| context.tcp_remove(accepted));
                return Err(Error::Mfile);
            };
            if let Err(error) = crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(
                SOCKET_FD_BASE + index as i32,
            ) {
                drop(sockets);
                let _removed = active_context(|context| context.tcp_remove(accepted));
                return Err(error);
            }
            sockets[index] = Some(SocketFd {
                owner_process_id: process_id,
                description: Arc::new(IrqSpinLock::new(descriptor)),
                close_on_exec: false,
            });
            return Ok((SOCKET_FD_BASE + index as i32, remote_address, remote_port));
        }
        if listener.nonblocking {
            return Err(Error::Again);
        }
        if now_ns().saturating_sub(started) >= CONNECT_TIMEOUT_NS {
            return Err(Error::TimedOut);
        }
        if wait_for_interrupt() && !restart_interrupted_io() {
            return Err(Error::Intr);
        }
    }
    Err(Error::TimedOut)
}

pub(crate) fn connect(
    file_descriptor: i32,
    remote_address: Ipv4Address,
    remote_port: u16,
) -> Result<(), Error> {
    if is_local_socket_fd(file_descriptor) {
        let _ = (remote_address, remote_port);
        return Err(Error::Inval);
    }
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
        if wait_for_interrupt() && !restart_interrupted_io() {
            return Err(Error::Intr);
        }
    }
    Err(Error::TimedOut)
}

pub(crate) fn send(file_descriptor: i32, bytes: &[u8]) -> Result<usize, Error> {
    if is_local_socket_fd(file_descriptor) {
        return send_to_local_socket_with_control(file_descriptor, None, bytes, None, false);
    }
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
    if is_local_socket_fd(file_descriptor) {
        return send_to_local_socket_with_control(file_descriptor, None, bytes, None, false);
    }
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
    if is_local_socket_fd(file_descriptor) {
        return receive_from_local_socket_with_control(file_descriptor, output, false, false, false)
            .map(|result| result.length);
    }
    receive_from(file_descriptor, output).map(|result| result.0)
}

pub(crate) fn receive_from(
    file_descriptor: i32,
    output: &mut [u8],
) -> Result<(usize, Option<(Ipv4Address, u16)>), Error> {
    if is_local_socket_fd(file_descriptor) {
        let result = receive_from_local_socket_with_control(
            file_descriptor,
            output,
            false,
            false,
            false,
        )?;
        return Ok((result.length, None));
    }
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
            if wait_for_interrupt() && !restart_interrupted_io() {
                return Err(Error::Intr);
            }
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
        if wait_for_interrupt() && !restart_interrupted_io() {
            return Err(Error::Intr);
        }
    }
    Err(Error::TimedOut)
}

pub(crate) fn send_to_local_socket(
    file_descriptor: i32,
    destination: Option<&[u8]>,
    bytes: &[u8],
) -> Result<usize, Error> {
    send_to_local_socket_with_control(file_descriptor, destination, bytes, None, false)
}

pub(crate) fn send_to_local_socket_with_control(
    file_descriptor: i32,
    destination: Option<&[u8]>,
    bytes: &[u8],
    mut control_message: Option<LocalControlMessage>,
    out_of_band: bool,
) -> Result<usize, Error> {
    let description = local_socket_description(file_descriptor)?;
    if let Some(destination) = destination {
        let (is_connected, peer_path) = {
            let description = description.lock();
            (description.connected, description.peer_path.clone())
        };
        if is_connected {
            if peer_path.as_deref() != Some(destination) {
                return Err(Error::Inval);
            }
        } else {
            connect_local_socket(file_descriptor, destination)?;
        }
    }
    let (pair, side, nonblocking, closed) = {
        let description = description.lock();
        (
            description.pair.as_ref().map(Arc::clone).ok_or(Error::NotConn)?,
            description.side,
            description.nonblocking,
            description.closed,
        )
    };
    if closed {
        return Err(Error::BadFd);
    }
    if out_of_band && bytes.len() != 1 {
        return Err(Error::Inval);
    }
    if out_of_band && control_message.is_some() {
        return Err(Error::OpNotSupp);
    }
    let sender_credentials = ThreadManager::current_process().ok().map(|process| {
        let user_ids = process.user_ids();
        let group_ids = process.group_ids();
        LocalCredentials {
            process_id: process.pid(),
            user_id: user_ids[1],
            group_id: group_ids[1],
        }
    });
    loop {
        ThreadManager::cancel_current_if_requested();
        let mut pair = pair.lock();
        let peer = 1 - side;
        if pair.readers[peer] == 0 || pair.read_shutdown[peer] {
            return Err(Error::Pipe);
        }
        if out_of_band {
            let position = pair.bytes_written[peer];
            let byte = *bytes.first().ok_or(Error::Inval)?;
            if pair.out_of_band[peer].is_some() {
                return Err(Error::Again);
            }
            if pair.out_of_band_inline[peer] {
                if pair.queues[peer].len() == LOCAL_SOCKET_BUFFER_SIZE {
                    drop(pair);
                    if nonblocking {
                        return Err(Error::Again);
                    }
                    if wait_for_interrupt() && !restart_interrupted_io() {
                        return Err(Error::Intr);
                    }
                    continue;
                }
                pair.queues[peer].push_back(byte);
                pair.bytes_written[peer] = pair.bytes_written[peer].saturating_add(1);
            }
            pair.out_of_band[peer] = Some((byte, position));
            return Ok(1);
        }
        if bytes.is_empty() && control_message.is_some() {
            return Err(Error::Inval);
        }
        let available = LOCAL_SOCKET_BUFFER_SIZE.saturating_sub(pair.queues[peer].len());
        if available == 0 {
            drop(pair);
            if nonblocking {
                return Err(Error::Again);
            }
            if wait_for_interrupt() && !restart_interrupted_io() {
                return Err(Error::Intr);
            }
            continue;
        }
        let count = bytes.len().min(available);
        let attach_credentials = pair.pass_credentials[peer];
        if control_message.is_some() || attach_credentials {
            pair.control_messages[peer].try_reserve(1).map_err(|_| Error::NoMem)?;
        }
        pair.queues[peer]
            .try_reserve(count)
            .map_err(|_| Error::NoMem)?;
        let queued_control = control_message.take().map(|mut message| {
            if attach_credentials && message.credentials.is_none() {
                message.credentials = sender_credentials;
            }
            message
        }).or_else(|| {
            attach_credentials.then(|| LocalControlMessage {
                rights: Vec::new(),
                credentials: sender_credentials,
            })
        });
        if let Some(message) = queued_control {
            let position = pair.bytes_written[peer];
            pair.control_messages[peer].push_back(QueuedControlMessage {
                position,
                message,
            });
        }
        pair.queues[peer].extend(bytes[..count].iter().copied());
        pair.bytes_written[peer] = pair.bytes_written[peer].saturating_add(count as u64);
        return Ok(count);
    }
}

pub(crate) fn receive_from_local_socket_with_control(
    file_descriptor: i32,
    output: &mut [u8],
    peek: bool,
    out_of_band: bool,
    receive_control: bool,
) -> Result<LocalReceiveResult, Error> {
    let description = local_socket_description(file_descriptor)?;
    let (pair, side, nonblocking, closed, inline) = {
        let description = description.lock();
        (
            description.pair.as_ref().map(Arc::clone).ok_or(Error::NotConn)?,
            description.side,
            description.nonblocking,
            description.closed,
            description.out_of_band_inline,
        )
    };
    if closed {
        return Err(Error::BadFd);
    }
    if output.is_empty() {
        return Ok(LocalReceiveResult {
            length: 0,
            control_messages: Vec::new(),
        });
    }
    loop {
        ThreadManager::cancel_current_if_requested();
        let mut discarded_rights = Vec::new();
        let attempt = {
            let mut pair = pair.lock();
            let reader = side;
            let writer = 1 - side;
            if out_of_band {
                if output.is_empty() {
                    return Ok(LocalReceiveResult {
                        length: 0,
                        control_messages: Vec::new(),
                    });
                }
                if inline {
                    return Err(Error::Inval);
                }
                if let Some((byte, _mark)) = pair.out_of_band[reader] {
                    output[0] = byte;
                    if !peek {
                        pair.out_of_band[reader] = None;
                    }
                    Some(LocalReceiveResult {
                        length: 1,
                        control_messages: Vec::new(),
                    })
                } else if pair.writers[writer] == 0 || pair.write_shutdown[writer] {
                    Some(LocalReceiveResult {
                        length: 0,
                        control_messages: Vec::new(),
                    })
                } else {
                    None
                }
            } else {
                let current_position = pair.bytes_read[reader];
                let ready_count = pair.control_messages[reader]
                    .iter()
                    .take_while(|message| message.position <= current_position)
                    .count();
                let mut control_messages = Vec::new();
                if receive_control {
                    control_messages
                        .try_reserve_exact(ready_count)
                        .map_err(|_| Error::NoMem)?;
                    if peek {
                        for message in pair.control_messages[reader]
                            .iter()
                            .take(ready_count)
                        {
                            control_messages.push(clone_local_control_message(&message.message)?);
                        }
                    } else {
                        for _ in 0..ready_count {
                            if let Some(message) = pair.control_messages[reader].pop_front() {
                                control_messages.push(message.message);
                            }
                        }
                    }
                } else if !peek {
                    for _ in 0..ready_count {
                        if let Some(message) = pair.control_messages[reader].pop_front() {
                            discarded_rights.extend(message.message.rights);
                        }
                    }
                }
                let next_control_position = pair.control_messages[reader]
                    .iter()
                    .find(|message| message.position > current_position)
                    .map(|message| message.position);
                let mut next_boundary = next_control_position;
                if !pair.out_of_band_inline[reader] {
                    if let Some((_byte, mark)) = pair.out_of_band[reader] {
                        if mark > current_position {
                            next_boundary = Some(next_boundary.map_or(mark, |boundary| boundary.min(mark)));
                        }
                    }
                }
                let urgent_blocked = !pair.out_of_band_inline[reader]
                    && pair.out_of_band[reader]
                        .is_some_and(|(_byte, mark)| current_position >= mark);
                let count = copy_local_queue(
                    &pair.queues[reader],
                    output,
                    current_position,
                    next_boundary,
                );
                if !peek {
                    for _ in 0..count {
                        let _ = pair.queues[reader].pop_front();
                    }
                    pair.bytes_read[reader] = pair.bytes_read[reader].saturating_add(count as u64);
                    if pair.out_of_band_inline[reader]
                        && pair.out_of_band[reader]
                            .is_some_and(|(_byte, mark)| pair.bytes_read[reader] > mark)
                    {
                        pair.out_of_band[reader] = None;
                    }
                }
                let eof = pair.writers[writer] == 0 || pair.write_shutdown[writer];
                (count != 0 || !control_messages.is_empty() || eof && !urgent_blocked)
                    .then_some(LocalReceiveResult {
                    length: count,
                    control_messages,
                })
            }
        };
        close_local_rights(discarded_rights);
        if let Some(result) = attempt {
            return Ok(result);
        }
        if nonblocking {
            return Err(Error::Again);
        }
        if wait_for_interrupt() && !restart_interrupted_io() {
            return Err(Error::Intr);
        }
    }
}

fn copy_local_queue(
    queue: &VecDeque<u8>,
    output: &mut [u8],
    current_position: u64,
    next_control_position: Option<u64>,
) -> usize {
    let mut count = output.len().min(queue.len());
    if let Some(position) = next_control_position {
        if position > current_position {
            count = count.min(usize::try_from(position - current_position).unwrap_or(usize::MAX));
        }
    }
    for (index, output_byte) in output.iter_mut().take(count).enumerate() {
        if let Some(byte) = queue.get(index) {
            *output_byte = *byte;
        }
    }
    count
}

fn clone_local_control_message(message: &LocalControlMessage) -> Result<LocalControlMessage, Error> {
    let mut rights = Vec::new();
    rights.try_reserve_exact(message.rights.len()).map_err(|_| Error::NoMem)?;
    rights.extend_from_slice(&message.rights);
    Ok(LocalControlMessage {
        rights,
        credentials: message.credentials,
    })
}

fn close_local_rights(rights: Vec<i32>) {
    let Ok(process) = ThreadManager::current_process() else {
        return;
    };
    close_local_rights_for_process(process.pid(), rights);
}

fn close_local_rights_for_process(process_id: i32, rights: Vec<i32>) {
    for descriptor in rights {
        let _ = crate::kernel::runtime::runtime_libc::filesystem::close_descriptor_for_process(
            process_id,
            descriptor,
        );
    }
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
    let process_id = ThreadManager::current_process()?.pid();
    close_for_process(process_id, file_descriptor)
}

pub(crate) fn close_for_process(process_id: i32, file_descriptor: i32) -> Result<(), Error> {
    if let Some(alias) = crate::kernel::runtime::runtime_libc::fd_alias::get_for_process(
        process_id,
        file_descriptor,
    ) {
        if alias.kind != crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind::Socket {
            return Err(Error::BadFd);
        }
        return close_direct_for_process(process_id, alias.backing);
    }
    if crate::kernel::runtime::runtime_libc::fd_alias::is_backing_for_process(
        process_id,
        file_descriptor,
    ) {
        return Err(Error::BadFd);
    }
    close_direct_for_process(process_id, file_descriptor)
}

pub(crate) fn close_direct_for_process(
    process_id: i32,
    file_descriptor: i32,
) -> Result<(), Error> {
    if local_socket_index_direct_for_process(process_id, file_descriptor).is_ok() {
        return close_local_socket_direct_for_process(process_id, file_descriptor);
    }
    let index = socket_index_direct_for_process(process_id, file_descriptor)?;
    let entry = SOCKETS.lock()[index].take().ok_or(Error::BadFd)?;
    let descriptor = {
        let mut description = entry.description.lock();
        description.descriptor_references = description.descriptor_references.saturating_sub(1);
        if description.descriptor_references != 0 {
            return Ok(());
        }
        *description
    };
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
    Ok(())
}

pub(crate) fn close_on_exec_descriptors() -> Result<(), Error> {
    let process_id = ThreadManager::current_process()?.pid();
    close_on_exec_descriptors_for_process(process_id)
}

pub(crate) fn close_on_exec_descriptors_for_process(process_id: i32) -> Result<(), Error> {
    let mut descriptors = Vec::new();
    {
        let sockets = SOCKETS.lock();
        descriptors
            .try_reserve(
                sockets
                    .iter()
                    .flatten()
                    .filter(|fd| fd.owner_process_id == process_id && fd.close_on_exec)
                    .count(),
            )
            .map_err(|_| Error::NoMem)?;
        descriptors.extend(sockets.iter().enumerate().filter_map(|(index, fd)| {
            fd.as_ref()
                .is_some_and(|fd| fd.owner_process_id == process_id && fd.close_on_exec)
                .then_some(SOCKET_FD_BASE + index as i32)
        }));
    }
    {
        let sockets = LOCAL_SOCKETS.lock();
        descriptors
            .try_reserve(
                sockets
                    .iter()
                    .flatten()
                    .filter(|fd| fd.owner_process_id == process_id && fd.close_on_exec)
                    .count(),
            )
            .map_err(|_| Error::NoMem)?;
        descriptors.extend(sockets.iter().enumerate().filter_map(|(index, fd)| {
            fd.as_ref()
                .is_some_and(|fd| fd.owner_process_id == process_id && fd.close_on_exec)
                .then_some(LOCAL_SOCKET_FD_BASE + index as i32)
        }));
    }
    {
        let devices = PACKET_DEVICES.lock();
        descriptors
            .try_reserve(
                devices
                    .iter()
                    .flatten()
                    .filter(|fd| fd.owner_process_id == process_id && fd.close_on_exec)
                    .count(),
            )
            .map_err(|_| Error::NoMem)?;
        descriptors.extend(devices.iter().enumerate().filter_map(|(index, fd)| {
            fd.as_ref()
                .is_some_and(|fd| fd.owner_process_id == process_id && fd.close_on_exec)
                .then_some(PACKET_FD_BASE + index as i32)
        }));
    }
    for descriptor in descriptors {
        let _ = close_for_process(process_id, descriptor)
            .or_else(|_| close_packet_fd_for_process(process_id, descriptor));
    }
    Ok(())
}

pub(crate) fn direct_descriptors_for_process(
    process_id: i32,
) -> Result<
    Vec<(
        i32,
        crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind,
        bool,
    )>,
    Error,
> {
    use crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind;

    let mut process_descriptors = Vec::new();
    {
        let sockets = SOCKETS.lock();
        process_descriptors
            .try_reserve_exact(
                sockets
                    .iter()
                    .flatten()
                    .filter(|descriptor| descriptor.owner_process_id == process_id)
                    .count(),
            )
            .map_err(|_| Error::NoMem)?;
        process_descriptors.extend(sockets.iter().enumerate().filter_map(|(index, descriptor)| {
            descriptor
                .as_ref()
                .filter(|descriptor| descriptor.owner_process_id == process_id)
                .map(|descriptor| {
                    (SOCKET_FD_BASE + index as i32, DescriptorKind::Socket, descriptor.close_on_exec)
                })
        }));
    }
    {
        let sockets = LOCAL_SOCKETS.lock();
        process_descriptors
            .try_reserve_exact(
                sockets
                    .iter()
                    .flatten()
                    .filter(|descriptor| descriptor.owner_process_id == process_id)
                    .count(),
            )
            .map_err(|_| Error::NoMem)?;
        process_descriptors.extend(sockets.iter().enumerate().filter_map(|(index, descriptor)| {
            descriptor
                .as_ref()
                .filter(|descriptor| descriptor.owner_process_id == process_id)
                .map(|descriptor| {
                    (
                        LOCAL_SOCKET_FD_BASE + index as i32,
                        DescriptorKind::Socket,
                        descriptor.close_on_exec,
                    )
                })
        }));
    }
    {
        let devices = PACKET_DEVICES.lock();
        process_descriptors
            .try_reserve_exact(
                devices
                    .iter()
                    .flatten()
                    .filter(|descriptor| descriptor.owner_process_id == process_id)
                    .count(),
            )
            .map_err(|_| Error::NoMem)?;
        process_descriptors.extend(devices.iter().enumerate().filter_map(|(index, descriptor)| {
            descriptor
                .as_ref()
                .filter(|descriptor| descriptor.owner_process_id == process_id)
                .map(|descriptor| {
                    (PACKET_FD_BASE + index as i32, DescriptorKind::Packet, descriptor.close_on_exec)
                })
        }));
    }
    Ok(process_descriptors)
}

fn available_socket_index(minimum: i32, process_id: i32) -> Result<usize, Error> {
    if minimum < 0 {
        return Err(Error::Inval);
    }
    let start = usize::try_from(minimum.saturating_sub(SOCKET_FD_BASE).max(0))
        .unwrap_or(SOCKET_COUNT)
        .min(SOCKET_COUNT);
    SOCKETS
        .lock()
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, descriptor)| {
            let target = SOCKET_FD_BASE + index as i32;
            (descriptor.is_none()
                && !crate::kernel::runtime::runtime_libc::fd_alias::contains_target_for_process(
                    process_id,
                    target,
                ))
            .then_some(index)
        })
        .ok_or(Error::Mfile)
}

pub(crate) fn duplicate_socket(
    file_descriptor: i32,
    minimum: i32,
    close_on_exec: bool,
) -> Result<i32, Error> {
    let process_id = ThreadManager::current_process()?.pid();
    duplicate_socket_internal(
        file_descriptor,
        minimum,
        close_on_exec,
        true,
        process_id,
        process_id,
    )
}

pub(crate) fn duplicate_socket_backing(file_descriptor: i32) -> Result<i32, Error> {
    let process_id = ThreadManager::current_process()?.pid();
    duplicate_socket_for_process(file_descriptor, process_id, process_id)
}

pub(crate) fn duplicate_socket_for_process(
    file_descriptor: i32,
    source_process_id: i32,
    target_process_id: i32,
) -> Result<i32, Error> {
    duplicate_socket_internal(
        file_descriptor,
        0,
        false,
        false,
        source_process_id,
        target_process_id,
    )
}

fn duplicate_socket_internal(
    file_descriptor: i32,
    minimum: i32,
    close_on_exec: bool,
    enforce_limit: bool,
    source_process_id: i32,
    target_process_id: i32,
) -> Result<i32, Error> {
    if minimum < 0 {
        return Err(Error::Inval);
    }
    if is_local_socket_fd_for_process(source_process_id, file_descriptor) {
        let source = local_socket_descriptor_for_process(source_process_id, file_descriptor)?;
        let start = usize::try_from(minimum.saturating_sub(LOCAL_SOCKET_FD_BASE).max(0))
            .unwrap_or(LOCAL_SOCKET_COUNT)
            .min(LOCAL_SOCKET_COUNT);
        let mut sockets = LOCAL_SOCKETS.lock();
        let target_index = sockets
            .iter()
            .enumerate()
            .skip(start)
            .find_map(|(index, descriptor)| {
                let target = LOCAL_SOCKET_FD_BASE + index as i32;
                (descriptor.is_none()
                    && !crate::kernel::runtime::runtime_libc::fd_alias::contains_target_for_process(
                        target_process_id,
                        target,
                    ))
                .then_some(index)
            })
            .ok_or(Error::Mfile)?;
        let target = LOCAL_SOCKET_FD_BASE + target_index as i32;
        if enforce_limit {
            crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(target)?;
        }
        {
            let mut description = source.description.lock();
            if description.closed {
                return Err(Error::BadFd);
            }
            description.descriptor_references = description
                .descriptor_references
                .checked_add(1)
                .ok_or(Error::Mfile)?;
        }
        sockets[target_index] = Some(LocalSocketFd {
            owner_process_id: target_process_id,
            description: source.description,
            close_on_exec,
        });
        return Ok(target);
    }
    let source_index = socket_index_for_process(source_process_id, file_descriptor)?;
    let source = SOCKETS.lock()[source_index].clone().ok_or(Error::BadFd)?;
    let target_index = available_socket_index(minimum, target_process_id)?;
    let target = SOCKET_FD_BASE + target_index as i32;
    if enforce_limit {
        crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(target)?;
    }
    {
        let mut description = source.description.lock();
        description.descriptor_references = description
            .descriptor_references
            .checked_add(1)
            .ok_or(Error::Mfile)?;
    }
    SOCKETS.lock()[target_index] = Some(SocketFd {
        owner_process_id: target_process_id,
        description: source.description,
        close_on_exec,
    });
    Ok(target)
}

pub(crate) fn duplicate_socket_to(
    file_descriptor: i32,
    target: i32,
    close_on_exec: bool,
) -> Result<i32, Error> {
    let process_id = ThreadManager::current_process()?.pid();
    if file_descriptor == target {
        return if close_on_exec { Err(Error::Inval) } else { Ok(target) };
    }
    if is_local_socket_fd_for_process(process_id, file_descriptor) {
        let target_index = local_socket_slot_index(target)?;
        let target_owner = LOCAL_SOCKETS.lock()[target_index]
            .as_ref()
            .map(|descriptor| descriptor.owner_process_id);
        if let Some(target_owner) = target_owner {
            if target_owner != process_id {
                return Err(Error::BadFd);
            }
            close_local_socket_for_process(process_id, target)?;
        }
        let source = local_socket_descriptor_for_process(process_id, file_descriptor)?;
        crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(target)?;
        let mut description = source.description.lock();
        if description.closed {
            return Err(Error::BadFd);
        }
        description.descriptor_references = description
            .descriptor_references
            .checked_add(1)
            .ok_or(Error::Mfile)?;
        drop(description);
        LOCAL_SOCKETS.lock()[target_index] = Some(LocalSocketFd {
            owner_process_id: process_id,
            description: source.description,
            close_on_exec,
        });
        return Ok(target);
    }
    let source_index = socket_index_for_process(process_id, file_descriptor)?;
    let target_index = socket_slot_index(target)?;
    let target_owner = SOCKETS.lock()[target_index]
        .as_ref()
        .map(|descriptor| descriptor.owner_process_id);
    if let Some(target_owner) = target_owner {
        if target_owner != process_id {
            return Err(Error::BadFd);
        }
        close_for_process(process_id, target)?;
    }
    crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(target)?;
    let source = SOCKETS.lock()[source_index].clone().ok_or(Error::BadFd)?;
    let mut description = source.description.lock();
    description.descriptor_references = description
        .descriptor_references
        .checked_add(1)
        .ok_or(Error::Mfile)?;
    drop(description);
    SOCKETS.lock()[target_index] = Some(SocketFd {
        owner_process_id: process_id,
        description: source.description,
        close_on_exec,
    });
    Ok(target)
}

pub(crate) fn descriptor_flags(file_descriptor: i32) -> Result<i32, Error> {
    if is_local_socket_fd(file_descriptor) {
        return Ok(i32::from(local_socket_descriptor(file_descriptor)?.close_on_exec));
    }
    let index = socket_index(file_descriptor)?;
    SOCKETS.lock()[index]
        .as_ref()
        .map(|descriptor| i32::from(descriptor.close_on_exec))
        .ok_or(Error::BadFd)
}

pub(crate) fn set_descriptor_flags(file_descriptor: i32, flags: i32) -> Result<(), Error> {
    if flags & !1 != 0 {
        return Err(Error::Inval);
    }
    if is_local_socket_fd(file_descriptor) {
        let index = local_socket_index(file_descriptor)?;
        LOCAL_SOCKETS.lock()[index]
            .as_mut()
            .ok_or(Error::BadFd)?
            .close_on_exec = flags & 1 != 0;
        return Ok(());
    }
    let index = socket_index(file_descriptor)?;
    SOCKETS.lock()[index]
        .as_mut()
        .ok_or(Error::BadFd)?
        .close_on_exec = flags & 1 != 0;
    Ok(())
}

fn close_local_socket(file_descriptor: i32) -> Result<(), Error> {
    let process_id = ThreadManager::current_process()?.pid();
    close_local_socket_for_process(process_id, file_descriptor)
}

fn close_local_socket_for_process(
    process_id: i32,
    file_descriptor: i32,
) -> Result<(), Error> {
    if let Some(alias) = crate::kernel::runtime::runtime_libc::fd_alias::get_for_process(
        process_id,
        file_descriptor,
    ) {
        if alias.kind != crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind::Socket {
            return Err(Error::BadFd);
        }
        return close_local_socket_direct_for_process(process_id, alias.backing);
    }
    if crate::kernel::runtime::runtime_libc::fd_alias::is_backing_for_process(
        process_id,
        file_descriptor,
    ) {
        return Err(Error::BadFd);
    }
    close_local_socket_direct_for_process(process_id, file_descriptor)
}

fn close_local_socket_direct_for_process(
    process_id: i32,
    file_descriptor: i32,
) -> Result<(), Error> {
    let index = local_socket_index_direct_for_process(process_id, file_descriptor)?;
    let descriptor = LOCAL_SOCKETS.lock()[index].take().ok_or(Error::BadFd)?;
    let mut close_rights = Vec::new();
    let (pair, side, listener, local_path) = {
        let mut description = descriptor.description.lock();
        description.descriptor_references = description.descriptor_references.saturating_sub(1);
        if description.descriptor_references != 0 {
            return Ok(());
        }
        description.closed = true;
        (
            description.pair.as_ref().map(Arc::clone),
            description.side,
            description.listener.as_ref().map(Arc::clone),
            description.local_path.clone(),
        )
    };
    if let Some(listener) = listener {
        let mut listener = listener.lock();
        listener.closed = true;
        while let Some(pending) = listener.pending.pop_front() {
            let mut pair = pending.pair.lock();
            pair.readers[1] = 0;
            pair.writers[1] = 0;
            for queued in pair.control_messages[1].drain(..) {
                close_rights.extend(queued.message.rights);
            }
        }
    }
    if let Some(pair) = pair {
        let mut pair = pair.lock();
        pair.readers[side] = pair.readers[side].saturating_sub(1);
        pair.writers[side] = pair.writers[side].saturating_sub(1);
        for queued in pair.control_messages[side].drain(..) {
            close_rights.extend(queued.message.rights);
        }
    }
    if let Some(path) = local_path {
        LOCAL_SOCKET_PATHS.lock().retain(|(registered, existing)| {
            registered != &path
                || existing
                    .upgrade()
                    .is_some_and(|existing| !Arc::ptr_eq(&existing, &descriptor.description))
        });
    }
    close_local_rights_for_process(process_id, close_rights);
    Ok(())
}

fn shutdown_local_socket(file_descriptor: i32, how: c_int) -> Result<(), Error> {
    if !matches!(how, 0..=2) {
        return Err(Error::Inval);
    }
    let description = local_socket_description(file_descriptor)?;
    let (pair, side, connected, closed) = {
        let description = description.lock();
        (
            description.pair.as_ref().map(Arc::clone),
            description.side,
            description.connected,
            description.closed,
        )
    };
    if closed {
        return Err(Error::BadFd);
    }
    if !connected {
        return Err(Error::NotConn);
    }
    let pair = pair.ok_or(Error::NotConn)?;
    let mut pair = pair.lock();
    if how != 1 {
        pair.read_shutdown[side] = true;
    }
    if how != 0 {
        pair.write_shutdown[side] = true;
    }
    Ok(())
}

fn set_local_socket_option(
    file_descriptor: i32,
    level: i32,
    option: i32,
    value: i32,
) -> Result<(), Error> {
    if level != SOL_SOCKET {
        return Err(Error::NoProtoOpt);
    }
    if !matches!(value, 0 | 1) {
        return Err(Error::Inval);
    }
    let description = local_socket_description(file_descriptor)?;
    let (pair, side) = {
        let mut description = description.lock();
        if description.closed {
            return Err(Error::BadFd);
        }
        match option {
            SO_PASSCRED => description.pass_credentials = value != 0,
            SO_OOBINLINE => description.out_of_band_inline = value != 0,
            _ => return Err(Error::NoProtoOpt),
        }
        (description.pair.as_ref().map(Arc::clone), description.side)
    };
    if let Some(pair) = pair {
        let mut pair = pair.lock();
        match option {
            SO_PASSCRED => pair.pass_credentials[side] = value != 0,
            SO_OOBINLINE => pair.out_of_band_inline[side] = value != 0,
            _ => return Err(Error::NoProtoOpt),
        }
    }
    Ok(())
}

fn get_local_socket_option(file_descriptor: i32, level: i32, option: i32) -> Result<i32, Error> {
    if level != SOL_SOCKET {
        return Err(Error::NoProtoOpt);
    }
    let description = local_socket_description(file_descriptor)?;
    let description = description.lock();
    if description.closed {
        return Err(Error::BadFd);
    }
    match option {
        SO_TYPE => Ok(SOCK_STREAM),
        SO_DOMAIN => Ok(AF_UNIX),
        SO_PROTOCOL => Ok(0),
        SO_ACCEPTCONN => Ok(i32::from(description.listener.is_some())),
        SO_ERROR => Ok(0),
        SO_SNDBUF | SO_RCVBUF => Ok(LOCAL_SOCKET_BUFFER_SIZE as i32),
        SO_PASSCRED => Ok(i32::from(description.pass_credentials)),
        SO_OOBINLINE => Ok(i32::from(description.out_of_band_inline)),
        _ => Err(Error::NoProtoOpt),
    }
}

fn local_socket_readiness(file_descriptor: i32, events: i16) -> Result<i16, Error> {
    const POLLIN: i16 = 0x001;
    const POLLOUT: i16 = 0x004;
    const POLLERR: i16 = 0x008;
    const POLLHUP: i16 = 0x010;
    let description = local_socket_description(file_descriptor)?;
    let (pair, side, listener, closed) = {
        let description = description.lock();
        (
            description.pair.as_ref().map(Arc::clone),
            description.side,
            description.listener.as_ref().map(Arc::clone),
            description.closed,
        )
    };
    if closed {
        return Err(Error::BadFd);
    }
    if let Some(listener) = listener {
        let listener = listener.lock();
        let mut readiness = 0;
        if events & POLLIN != 0 && !listener.pending.is_empty() {
            readiness |= POLLIN;
        }
        if listener.closed {
            readiness |= POLLHUP | POLLERR;
        }
        return Ok(readiness);
    }
    let pair = pair.ok_or(Error::NotConn)?;
    let pair = pair.lock();
    let peer = 1 - side;
    let readable = pair.read_shutdown[side]
        || !pair.queues[side].is_empty()
        || pair.out_of_band[side].is_some()
        || pair.writers[peer] == 0
        || pair.write_shutdown[peer];
    let writable = pair.writers[side] != 0
        && !pair.write_shutdown[side]
        && pair.readers[peer] != 0
        && !pair.read_shutdown[peer]
        && pair.queues[peer].len() < LOCAL_SOCKET_BUFFER_SIZE;
    let mut readiness = 0;
    if events & POLLIN != 0 && readable {
        readiness |= POLLIN;
    }
    if events & POLLOUT != 0 && writable {
        readiness |= POLLOUT;
    }
    if pair.readers[peer] == 0 || pair.read_shutdown[peer] {
        readiness |= POLLERR;
    }
    if pair.readers[peer] == 0 && pair.writers[peer] == 0 {
        readiness |= POLLHUP;
    }
    Ok(readiness)
}

pub(crate) fn socket_at_mark(file_descriptor: i32) -> Result<bool, Error> {
    if !is_local_socket_fd(file_descriptor) {
        return Err(Error::OpNotSupp);
    }
    let description = local_socket_description(file_descriptor)?;
    let (pair, side, inline, closed) = {
        let description = description.lock();
        (
            description.pair.as_ref().map(Arc::clone),
            description.side,
            description.out_of_band_inline,
            description.closed,
        )
    };
    if closed {
        return Err(Error::BadFd);
    }
    let pair = pair.ok_or(Error::NotConn)?;
    let pair = pair.lock();
    let Some((_byte, mark)) = pair.out_of_band[side] else {
        return Ok(false);
    };
    let _ = inline;
    Ok(pair.bytes_read[side] == mark)
}

pub(crate) fn shutdown(file_descriptor: i32, how: c_int) -> Result<(), Error> {
    if is_local_socket_fd(file_descriptor) {
        return shutdown_local_socket(file_descriptor, how);
    }
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
    if is_local_socket_fd(file_descriptor) {
        return Ok(local_socket_description(file_descriptor)?.lock().nonblocking);
    }
    Ok(socket_descriptor(socket_index(file_descriptor)?)?.nonblocking)
}

pub(crate) fn set_nonblocking(file_descriptor: i32, nonblocking: bool) -> Result<(), Error> {
    if is_local_socket_fd(file_descriptor) {
        local_socket_description(file_descriptor)?.lock().nonblocking = nonblocking;
        return Ok(());
    }
    let index = socket_index(file_descriptor)?;
    let mut descriptor = socket_descriptor(index)?;
    descriptor.nonblocking = nonblocking;
    set_socket_descriptor(index, descriptor);
    Ok(())
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
    if is_local_socket_fd(file_descriptor) {
        return set_local_socket_option(file_descriptor, level, option, value);
    }
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
    if is_local_socket_fd(file_descriptor) {
        return get_local_socket_option(file_descriptor, level, option);
    }
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
    if is_local_socket_fd(file_descriptor) {
        return local_socket_readiness(file_descriptor, events);
    }
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
    })?;
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
    let process_id = ThreadManager::current_process()?.pid();
    socket_index_for_process(process_id, file_descriptor)
}

fn socket_index_for_process(process_id: i32, file_descriptor: i32) -> Result<usize, Error> {
    let file_descriptor = crate::kernel::runtime::runtime_libc::fd_alias::resolve_for_process(
        process_id,
        file_descriptor,
        crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind::Socket,
    )?;
    socket_index_direct_for_process(process_id, file_descriptor)
}

fn socket_index_direct_for_process(process_id: i32, file_descriptor: i32) -> Result<usize, Error> {
    let index = socket_slot_index(file_descriptor)?;
    if !SOCKETS.lock()[index]
        .as_ref()
        .is_some_and(|descriptor| descriptor.owner_process_id == process_id)
    {
        return Err(Error::BadFd);
    }
    Ok(index)
}

fn socket_slot_index(file_descriptor: i32) -> Result<usize, Error> {
    let index = file_descriptor
        .checked_sub(SOCKET_FD_BASE)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(Error::BadFd)?;
    (index < SOCKET_COUNT).then_some(index).ok_or(Error::BadFd)
}

fn packet_index(file_descriptor: i32) -> Result<usize, Error> {
    let process_id = ThreadManager::current_process()?.pid();
    packet_index_for_process(process_id, file_descriptor)
}

fn packet_index_for_process(process_id: i32, file_descriptor: i32) -> Result<usize, Error> {
    let file_descriptor = crate::kernel::runtime::runtime_libc::fd_alias::resolve_for_process(
        process_id,
        file_descriptor,
        crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind::Packet,
    )?;
    packet_index_direct_for_process(process_id, file_descriptor)
}

fn packet_index_direct_for_process(process_id: i32, file_descriptor: i32) -> Result<usize, Error> {
    let index = packet_slot_index(file_descriptor)?;
    if !PACKET_DEVICES.lock()[index]
        .as_ref()
        .is_some_and(|descriptor| descriptor.owner_process_id == process_id)
    {
        return Err(Error::BadFd);
    }
    Ok(index)
}

fn packet_slot_index(file_descriptor: i32) -> Result<usize, Error> {
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

fn packet_descriptor_for_process(
    process_id: i32,
    file_descriptor: i32,
) -> Result<PacketDescriptor, Error> {
    PACKET_DEVICES.lock()[packet_index_for_process(process_id, file_descriptor)?]
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
    let description = SOCKETS
        .lock()
        .get(index)
        .and_then(Option::as_ref)
        .map(|descriptor| Arc::clone(&descriptor.description))
        .ok_or(Error::BadFd)?;
    let descriptor = *description.lock();
    Ok(descriptor)
}

fn set_socket_descriptor(index: usize, descriptor: SocketDescriptor) {
    let description = SOCKETS.lock()[index]
        .as_ref()
        .map(|descriptor| Arc::clone(&descriptor.description));
    if let Some(description) = description {
        *description.lock() = descriptor;
    }
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
            .any(|descriptor| descriptor.description.lock().local_port == port);
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

fn wait_for_interrupt() -> bool {
    if ThreadManager::current_signal_wake_pending() {
        return true;
    }
    crate::kernel::runtime::runtime_ffi::kz_host_wait_for_interrupt();
    ThreadManager::current_signal_wake_pending()
}

fn restart_interrupted_io() -> bool {
    crate::kernel::runtime::runtime_posix::signal::dispatch_pending_signals().unwrap_or(true)
}