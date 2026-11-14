/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::kernel::arch::traits::CpuArchitecture as _;
use crate::kernel::error::Error;
use crate::kernel::net::tcp::segment::{
    TcpSegment, TCP_HEADER_LENGTH, TCP_FLAG_ACK, TCP_FLAG_FIN, TCP_FLAG_PSH, TCP_FLAG_RST,
    TCP_FLAG_SYN,
};
use crate::kernel::net::wire::Ipv4Address;

const MAX_TCP_RECEIVE_WINDOW: usize = u16::MAX as usize;
const MAX_TCP_SOCKETS: usize = 256;
const MAX_TCP_ACCEPT_BACKLOG: usize = 128;
const MAX_TCP_OUT_OF_ORDER_SEGMENTS: usize = 64;
const MAX_TCP_OUTSTANDING_SEGMENTS: usize = 64;
const MAX_TCP_SEND_BUFFER_PER_SOCKET: usize = 64 * 1024;
const MAX_TCP_SEND_BUFFER_TOTAL: usize = 4 * 1024 * 1024;
const INITIAL_RETRANSMIT_TIMEOUT_NS: u64 = 1_000_000_000;
const MAXIMUM_RETRANSMIT_TIMEOUT_NS: u64 = 60_000_000_000;
const MAXIMUM_RETRANSMISSIONS: u8 = 5;
const TCP_CLOSE_TIMEOUT_NS: u64 = 120_000_000_000;
const TCP_TIME_WAIT_NS: u64 = 60_000_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TcpState {
    Closed,
    Listen,
    SynReceived,
    SynSent,
    Established,
    FinWait1,
    FinWait2,
    CloseWait,
    Closing,
    TimeWait,
    LastAck,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TcpSocketHandle {
    index: usize,
    generation: u32,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct TcpFlowKey {
    local_address: Ipv4Address,
    remote_address: Ipv4Address,
    local_port: u16,
    remote_port: u16,
}

struct TcpFlowBucket {
    key: TcpFlowKey,
    //@FIXME: INDEXTYPE(handles); per-flow handle list uses O(n) membership/search; add a hash side index for expected O(1) lookup if fan-out grows.
    handles: Vec<TcpSocketHandle>,
}

struct TcpListener {
    address: Ipv4Address,
    handle: TcpSocketHandle,
}

struct TcpListenerBucket {
    port: u16,
    //@FIXME: INDEXTYPE(listeners); per-port exact/wildcard listener list is scanned in O(n); keep Vec for typically small fan-out or add an address index if hot.
    listeners: Vec<TcpListener>,
}

struct OutstandingSegment {
    sequence_number: u32,
    end_sequence_number: u32,
    flags: u8,
    //@FIXME: INDEXTYPE(payload); contiguous segment bytes; Vec indexing is O(1), keep Vec.
    payload: Vec<u8>,
}

struct ReassemblySegment {
    sequence_number: u32,
    //@FIXME: INDEXTYPE(payload); contiguous reassembly bytes; Vec indexing is O(1), keep Vec.
    payload: Vec<u8>,
    fin: bool,
}

struct TcpSocket {
    state: TcpState,
    local_address: Ipv4Address,
    remote_address: Ipv4Address,
    local_port: u16,
    remote_port: u16,
    send_unacknowledged: u32,
    send_next: u32,
    receive_next: Option<u32>,
    peer_window: u16,
    send_buffer: VecDeque<u8>,
    receive_buffer: VecDeque<u8>,
    //@FIXME: INDEXTYPE(out_of_order); sequence-ordered reassembly list capped at 64; O(n) scans/removal are bounded, keep Vec.
    out_of_order: Vec<ReassemblySegment>,
    out_of_order_bytes: usize,
    maximum_segment_size: usize,
    congestion_window_bytes: usize,
    slow_start_threshold_bytes: usize,
    outstanding: VecDeque<OutstandingSegment>,
    outstanding_payload_bytes: usize,
    retransmit_at_ns: u64,
    retransmit_timeout_ns: u64,
    retransmissions: u8,
    close_requested: bool,
    orphaned: bool,
    flow_indexed: bool,
    listener_indexed: bool,
    close_deadline_ns: Option<u64>,
    parent_listener: Option<TcpSocketHandle>,
    accept_queue: VecDeque<TcpSocketHandle>,
    backlog: usize,
}

struct SocketSlot {
    generation: u32,
    socket: Option<TcpSocket>,
}

pub(crate) struct TcpTransmission {
    pub(crate) source_address: Ipv4Address,
    pub(crate) destination_address: Ipv4Address,
    pub(crate) source_port: u16,
    pub(crate) destination_port: u16,
    pub(crate) sequence_number: u32,
    pub(crate) acknowledgment_number: u32,
    pub(crate) flags: u8,
    pub(crate) window: u16,
    //@FIXME: INDEXTYPE(payload); owned transmission byte buffer; Vec indexing is O(1) and append amortized O(1), keep Vec.
    pub(crate) payload: Vec<u8>,
}

pub(crate) struct TcpSocketTable {
    //@FIXME: INDEXTYPE(slots); generation-checked direct-index socket table; Vec access is O(1), add a free-slot stack if vacant-slot scans become costly.
    slots: Vec<SocketSlot>,
    //@FIXME: INDEXTYPE(flows); sorted key index has O(log n) lookup but O(n) updates; use a balanced tree/map for O(log n) updates if iteration requirements permit.
    flows: Vec<TcpFlowBucket>,
    //@FIXME: INDEXTYPE(listeners); port-sorted index has O(log n) lookup but O(n) updates; use a balanced tree/map for O(log n) updates if iteration requirements permit.
    listeners: Vec<TcpListenerBucket>,
    //@FIXME: INDEXTYPE(removed_indexes); reserved deferred-removal batch, appended then drained; keep Vec.
    removed_indexes: Vec<(TcpSocketHandle, Option<TcpFlowKey>, Option<u16>)>,
    maximum_segment_size: usize,
    queued_send_bytes: usize,
}

impl TcpSocketTable {
    pub(crate) fn new(interface_mtu: usize) -> Option<Self> {
        let maximum_segment_size = interface_mtu
            .saturating_sub(20 + TCP_HEADER_LENGTH)
            .min(usize::from(u16::MAX));
        if maximum_segment_size == 0 {
            return None;
        }
        let mut removed_indexes = Vec::new();
        removed_indexes.try_reserve_exact(MAX_TCP_SOCKETS).ok()?;
        Some(Self {
            slots: Vec::new(),
            flows: Vec::new(),
            listeners: Vec::new(),
            removed_indexes,
            maximum_segment_size,
            queued_send_bytes: 0,
        })
    }

    fn add_flow(&mut self, key: TcpFlowKey, handle: TcpSocketHandle) -> Result<(), Error> {
        match self.flows.binary_search_by_key(&key, |bucket| bucket.key) {
            Ok(index) => {
                if self.flows[index].handles.iter().copied().any(|existing| {
                    existing != handle
                        && self.slots.get(existing.index).is_some_and(|slot| {
                            slot.generation == existing.generation
                                && slot.socket.as_ref().is_some_and(|socket| {
                                    socket.state != TcpState::Closed
                                        && socket.state != TcpState::Listen
                                })
                        })
                }) {
                    return Err(Error::AddrInUse);
                }
                self.flows[index]
                    .handles
                    .try_reserve(1)
                    .map_err(|_| Error::NoMem)?;
                self.flows[index].handles.push(handle);
            }
            Err(index) => {
                let mut handles = Vec::new();
                handles.try_reserve_exact(1).map_err(|_| Error::NoMem)?;
                handles.push(handle);
                self.flows.try_reserve(1).map_err(|_| Error::NoMem)?;
                self.flows.insert(index, TcpFlowBucket { key, handles });
            }
        }
        if let Some(socket) = self.socket_mut(handle) {
            socket.flow_indexed = true;
        }
        Ok(())
    }

    fn remove_flow(&mut self, key: TcpFlowKey, handle: TcpSocketHandle) {
        let Ok(bucket_index) = self.flows.binary_search_by_key(&key, |bucket| bucket.key) else {
            return;
        };
        let bucket = &mut self.flows[bucket_index];
        if let Some(index) = bucket.handles.iter().position(|candidate| *candidate == handle) {
            bucket.handles.remove(index);
        }
        if bucket.handles.is_empty() {
            self.flows.remove(bucket_index);
        }
        if let Some(socket) = self.socket_mut(handle) {
            socket.flow_indexed = false;
        }
    }

    fn flow_handle(&self, key: TcpFlowKey) -> Option<TcpSocketHandle> {
        let bucket_index = self.flows.binary_search_by_key(&key, |bucket| bucket.key).ok()?;
        self.flows[bucket_index].handles.iter().copied().find(|handle| {
            self.slots.get(handle.index).is_some_and(|slot| {
                slot.generation == handle.generation
                    && slot.socket.as_ref().is_some_and(|socket| {
                        socket.state != TcpState::Listen && socket.state != TcpState::Closed
                    })
            })
        })
    }

    fn listener_conflicts(&self, address: Ipv4Address, port: u16) -> bool {
        let Ok(bucket_index) = self.listeners.binary_search_by_key(&port, |bucket| bucket.port) else {
            return false;
        };
        self.listeners[bucket_index].listeners.iter().any(|listener| {
            address == Ipv4Address([0; 4])
                || listener.address == Ipv4Address([0; 4])
                || listener.address == address
        })
    }

    fn add_listener(&mut self, address: Ipv4Address, port: u16, handle: TcpSocketHandle) -> Result<(), Error> {
        match self.listeners.binary_search_by_key(&port, |bucket| bucket.port) {
            Ok(index) => {
                self.listeners[index]
                    .listeners
                    .try_reserve(1)
                    .map_err(|_| Error::NoMem)?;
                self.listeners[index].listeners.push(TcpListener { address, handle });
            }
            Err(index) => {
                let mut listeners = Vec::new();
                listeners.try_reserve_exact(1).map_err(|_| Error::NoMem)?;
                listeners.push(TcpListener { address, handle });
                self.listeners.try_reserve(1).map_err(|_| Error::NoMem)?;
                self.listeners.insert(index, TcpListenerBucket { port, listeners });
            }
        }
        if let Some(socket) = self.socket_mut(handle) {
            socket.listener_indexed = true;
        }
        Ok(())
    }

    fn remove_listener(&mut self, port: u16, handle: TcpSocketHandle) {
        let Ok(bucket_index) = self.listeners.binary_search_by_key(&port, |bucket| bucket.port) else {
            return;
        };
        let bucket = &mut self.listeners[bucket_index];
        if let Some(index) = bucket.listeners.iter().position(|listener| listener.handle == handle) {
            bucket.listeners.remove(index);
        }
        if bucket.listeners.is_empty() {
            self.listeners.remove(bucket_index);
        }
        if let Some(socket) = self.socket_mut(handle) {
            socket.listener_indexed = false;
        }
    }

    fn listener_handle(&self, address: Ipv4Address, port: u16) -> Option<TcpSocketHandle> {
        let bucket_index = self.listeners.binary_search_by_key(&port, |bucket| bucket.port).ok()?;
        let listeners = &self.listeners[bucket_index].listeners;
        listeners
            .iter()
            .find(|listener| listener.address == address)
            .or_else(|| listeners.iter().find(|listener| listener.address == Ipv4Address([0; 4])))
            .map(|listener| listener.handle)
    }

    fn allocate_slot(&mut self) -> Result<(usize, u32), Error> {
        let index = if let Some(index) = self.slots.iter().position(|slot| slot.socket.is_none()) {
            index
        } else {
            if self.slots.len() >= MAX_TCP_SOCKETS {
                return Err(Error::NoBufs);
            }
            self.slots.try_reserve(1).map_err(|_| Error::NoMem)?;
            self.slots.push(SocketSlot {
                generation: 0,
                socket: None,
            });
            self.slots.len() - 1
        };
        let generation = self.slots[index].generation.wrapping_add(1).max(1);
        self.slots[index].generation = generation;
        Ok((index, generation))
    }

    fn outstanding_send_bytes(&self) -> usize {
        self.slots
            .iter()
            .filter_map(|slot| slot.socket.as_ref())
            .fold(0usize, |total, socket| {
                total.saturating_add(socket.outstanding_payload_bytes)
            })
    }

    pub(crate) fn connect(
        &mut self,
        local_address: Ipv4Address,
        remote_address: Ipv4Address,
        local_port: u16,
        remote_port: u16,
        _now_ns: u64,
    ) -> Result<TcpSocketHandle, Error> {
        if local_port == 0 || remote_port == 0 {
            return Err(Error::Inval);
        }
        let sequence_number =
            crate::kernel::arch::CpuArchitecture::random_u32().ok_or(Error::NoSys)?;
        self.connect_with_initial_sequence(
            local_address,
            remote_address,
            local_port,
            remote_port,
            sequence_number,
        )
    }

    fn connect_with_initial_sequence(
        &mut self,
        local_address: Ipv4Address,
        remote_address: Ipv4Address,
        local_port: u16,
        remote_port: u16,
        sequence_number: u32,
    ) -> Result<TcpSocketHandle, Error> {
        let (index, generation) = self.allocate_slot()?;
        let socket = TcpSocket::new(
            local_address,
            remote_address,
            local_port,
            remote_port,
            sequence_number,
            TcpState::SynSent,
            self.maximum_segment_size,
        )?;
        self.slots[index].socket = Some(socket);
        let handle = TcpSocketHandle { index, generation };
        if let Err(error) = self.add_flow(
            TcpFlowKey {
                local_address,
                remote_address,
                local_port,
                remote_port,
            },
            handle,
        ) {
            self.slots[index].socket = None;
            return Err(error);
        }
        Ok(handle)
    }

    pub(crate) fn listen(
        &mut self,
        local_address: Ipv4Address,
        local_port: u16,
        backlog: usize,
        _now_ns: u64,
    ) -> Result<TcpSocketHandle, Error> {
        if local_port == 0 {
            return Err(Error::Inval);
        }
        if self.listener_conflicts(local_address, local_port) {
            return Err(Error::Exist);
        }
        let (index, generation) = self.allocate_slot()?;
        let mut socket = TcpSocket::new(
            local_address,
            Ipv4Address([0; 4]),
            local_port,
            0,
            0,
            TcpState::Listen,
            self.maximum_segment_size,
        )?;
        socket.backlog = backlog.clamp(1, MAX_TCP_ACCEPT_BACKLOG);
        socket
            .accept_queue
            .try_reserve_exact(socket.backlog)
            .map_err(|_| Error::NoMem)?;
        self.slots[index].socket = Some(socket);
        let handle = TcpSocketHandle { index, generation };
        if let Err(error) = self.add_listener(local_address, local_port, handle) {
            self.slots[index].socket = None;
            return Err(error);
        }
        Ok(handle)
    }

    pub(crate) fn accept(
        &mut self,
        listener: TcpSocketHandle,
    ) -> Result<Option<TcpSocketHandle>, Error> {
        loop {
            let child = {
                let socket = self.socket_mut(listener).ok_or(Error::BadFd)?;
                if socket.state != TcpState::Listen {
                    return Err(Error::Inval);
                }
                socket.accept_queue.pop_front()
            };
            let Some(child) = child else {
                return Ok(None);
            };
            let Some(socket) = self.socket_mut(child) else {
                continue;
            };
            if socket.state == TcpState::Closed {
                continue;
            }
            socket.parent_listener = None;
            return Ok(Some(child));
        }
    }

    pub(crate) fn state(&self, handle: TcpSocketHandle) -> Option<TcpState> {
        self.socket(handle).map(|socket| socket.state)
    }

    pub(crate) fn readable(&self, handle: TcpSocketHandle) -> bool {
        let Some(socket) = self.socket(handle) else {
            return false;
        };
        match socket.state {
            TcpState::Listen => socket.accept_queue.iter().any(|child_handle| {
                self.socket(*child_handle).is_some_and(|child| {
                    child.state != TcpState::Closed
                        && child.parent_listener == Some(handle)
                })
            }),
            TcpState::CloseWait | TcpState::Closing | TcpState::TimeWait | TcpState::Closed => true,
            _ => !socket.receive_buffer.is_empty(),
        }
    }

    pub(crate) fn writable(&self, handle: TcpSocketHandle) -> bool {
        let Some(socket) = self.socket(handle) else {
            return false;
        };
        if socket.state != TcpState::Established {
            return false;
        }
        let socket_send_bytes = socket
            .send_buffer
            .len()
            .saturating_add(socket.outstanding_payload_bytes);
        let total_send_bytes = self
            .queued_send_bytes
            .saturating_add(self.outstanding_send_bytes());
        socket_send_bytes < MAX_TCP_SEND_BUFFER_PER_SOCKET
            && total_send_bytes < MAX_TCP_SEND_BUFFER_TOTAL
    }

    pub(crate) fn local_endpoint(
        &self,
        handle: TcpSocketHandle,
    ) -> Option<(Ipv4Address, u16)> {
        let socket = self.socket(handle)?;
        Some((socket.local_address, socket.local_port))
    }

    pub(crate) fn peer_endpoint(
        &self,
        handle: TcpSocketHandle,
    ) -> Option<(Ipv4Address, u16)> {
        let socket = self.socket(handle)?;
        (socket.remote_port != 0).then_some((socket.remote_address, socket.remote_port))
    }

    pub(crate) fn send(
        &mut self,
        handle: TcpSocketHandle,
        bytes: &[u8],
    ) -> Result<usize, Error> {
        let socket = self.socket(handle).ok_or(Error::BadFd)?;
        if socket.state != TcpState::Established {
            return Err(Error::Busy);
        }
        let socket_send_bytes = socket
            .send_buffer
            .len()
            .saturating_add(socket.outstanding_payload_bytes);
        let total_send_bytes = self
            .queued_send_bytes
            .saturating_add(self.outstanding_send_bytes());
        let accepted = bytes
            .len()
            .min(MAX_TCP_SEND_BUFFER_PER_SOCKET.saturating_sub(socket_send_bytes))
            .min(MAX_TCP_SEND_BUFFER_TOTAL.saturating_sub(total_send_bytes));
        if accepted == 0 && !bytes.is_empty() {
            return Err(Error::NoBufs);
        }
        let socket = self.socket_mut(handle).ok_or(Error::BadFd)?;
        socket.send_buffer
            .try_reserve(accepted)
            .map_err(|_| Error::NoMem)?;
        socket.send_buffer.extend(bytes[..accepted].iter().copied());
        self.queued_send_bytes += accepted;
        Ok(accepted)
    }

    pub(crate) fn receive(
        &mut self,
        handle: TcpSocketHandle,
        output: &mut [u8],
    ) -> Result<usize, Error> {
        let socket = self.socket_mut(handle).ok_or(Error::BadFd)?;
        let length = output.len().min(socket.receive_buffer.len());
        for byte in &mut output[..length] {
            if let Some(value) = socket.receive_buffer.pop_front() {
                *byte = value;
            }
        }
        Ok(length)
    }

    pub(crate) fn close(&mut self, handle: TcpSocketHandle, now_ns: u64) -> Result<(), Error> {
        let index = self.slot_index(handle)?;
        let (port, is_listener) = {
            let socket = self.slots[index].socket.as_ref().ok_or(Error::BadFd)?;
            (socket.local_port, socket.state == TcpState::Listen)
        };
        let socket = self.socket_mut(handle).ok_or(Error::BadFd)?;
        socket.orphaned = true;
        let close_timeout = if socket.state == TcpState::TimeWait {
            TCP_TIME_WAIT_NS
        } else {
            TCP_CLOSE_TIMEOUT_NS
        };
        socket.close_deadline_ns = Some(now_ns.saturating_add(close_timeout));
        Self::request_close(socket);
        if is_listener {
            self.remove_listener(port, handle);
            for slot in &mut self.slots {
                let Some(child) = slot.socket.as_mut() else {
                    continue;
                };
                if child.parent_listener == Some(handle) {
                    child.state = TcpState::Closed;
                    child.orphaned = true;
                    child.close_deadline_ns = Some(now_ns);
                    child.outstanding.clear();
                    child.outstanding_payload_bytes = 0;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn shutdown_write(&mut self, handle: TcpSocketHandle) -> Result<(), Error> {
        let socket = self.socket_mut(handle).ok_or(Error::BadFd)?;
        Self::request_close(socket);
        Ok(())
    }

    fn request_close(socket: &mut TcpSocket) {
        match socket.state {
            TcpState::Listen | TcpState::SynSent => socket.state = TcpState::Closed,
            TcpState::Established | TcpState::CloseWait => socket.close_requested = true,
            _ => {}
        }
    }

    pub(crate) fn remove(&mut self, handle: TcpSocketHandle) -> bool {
        let (buffered_send_bytes, flow_key, listener_port, is_listener) = {
            let Some(slot) = self.slots.get_mut(handle.index) else {
                return false;
            };
            if slot.generation != handle.generation || slot.socket.is_none() {
                return false;
            }
            let socket = slot.socket.as_ref().unwrap();
            let buffered_send_bytes = socket.send_buffer.len();
            let flow_key = (socket.remote_port != 0).then_some(TcpFlowKey {
                local_address: socket.local_address,
                remote_address: socket.remote_address,
                local_port: socket.local_port,
                remote_port: socket.remote_port,
            });
            let listener_port = (socket.state == TcpState::Listen || socket.remote_port == 0)
                .then_some(socket.local_port);
            let is_listener = socket.state == TcpState::Listen || socket.remote_port == 0;
            slot.socket = None;
            (buffered_send_bytes, flow_key, listener_port, is_listener)
        };
        if let Some(key) = flow_key {
            self.remove_flow(key, handle);
        }
        if let Some(port) = listener_port {
            self.remove_listener(port, handle);
        }
        if is_listener {
            for slot in &mut self.slots {
                let Some(child) = slot.socket.as_mut() else {
                    continue;
                };
                if child.parent_listener == Some(handle) {
                    child.state = TcpState::Closed;
                    child.orphaned = true;
                    child.outstanding.clear();
                    child.outstanding_payload_bytes = 0;
                }
            }
        }
        self.queued_send_bytes = self.queued_send_bytes.saturating_sub(buffered_send_bytes);
        true
    }

    pub(crate) fn poll(
        &mut self,
        now_ns: u64,
        transmissions: &mut Vec<TcpTransmission>,
    ) -> Result<bool, Error> {
        let maximum_segment_size = self.maximum_segment_size;
        transmissions
            .try_reserve(self.slots.len())
            .map_err(|_| Error::NoMem)?;
        let mut removed_indexes = core::mem::take(&mut self.removed_indexes);
        let mut changed = false;
        let mut removed_send_bytes = 0usize;
        let poll_result = (|| -> Result<(), Error> {
            for (slot_index, slot) in self.slots.iter_mut().enumerate() {
                let Some(socket) = slot.socket.as_mut() else {
                    continue;
                };
                if socket.orphaned
                    && socket
                        .close_deadline_ns
                        .is_some_and(|deadline| now_ns >= deadline)
                {
                    socket.state = TcpState::Closed;
                    socket.outstanding.clear();
                    socket.outstanding_payload_bytes = 0;
                }
                if socket.state == TcpState::TimeWait
                    && socket
                        .close_deadline_ns
                        .is_some_and(|deadline| now_ns >= deadline)
                {
                    socket.state = TcpState::Closed;
                }
                if socket.state == TcpState::Closed {
                    let handle = TcpSocketHandle {
                        index: slot_index,
                        generation: slot.generation,
                    };
                    let flow_key = if socket.flow_indexed {
                        socket.flow_indexed = false;
                        Some(TcpFlowKey {
                            local_address: socket.local_address,
                            remote_address: socket.remote_address,
                            local_port: socket.local_port,
                            remote_port: socket.remote_port,
                        })
                    } else {
                        None
                    };
                    let listener_port = if socket.listener_indexed {
                        socket.listener_indexed = false;
                        Some(socket.local_port)
                    } else {
                        None
                    };
                    if flow_key.is_some() || listener_port.is_some() || socket.orphaned {
                        removed_indexes.push((handle, flow_key, listener_port));
                    }
                    if socket.orphaned {
                        removed_send_bytes =
                            removed_send_bytes.saturating_add(socket.send_buffer.len());
                        slot.socket = None;
                    }
                    continue;
                }
                if !socket.outstanding.is_empty() && now_ns >= socket.retransmit_at_ns {
                    if socket.retransmissions == MAXIMUM_RETRANSMISSIONS {
                        socket.state = TcpState::Closed;
                        if socket.parent_listener.is_some() {
                            socket.orphaned = true;
                            socket.close_deadline_ns = Some(now_ns);
                        }
                        socket.outstanding.clear();
                        socket.outstanding_payload_bytes = 0;
                        changed = true;
                        continue;
                    }
                    if socket.retransmissions == 0 {
                        socket.on_retransmission_timeout();
                    }
                    let outstanding = socket.outstanding.front().ok_or(Error::Proto)?;
                    let retransmission = socket.transmission(
                        outstanding.sequence_number,
                        outstanding.flags,
                        &outstanding.payload,
                    )?;
                    transmissions.try_reserve(1).map_err(|_| Error::NoMem)?;
                    transmissions.push(retransmission);
                    socket.retransmissions += 1;
                    socket.retransmit_timeout_ns = socket
                        .retransmit_timeout_ns
                        .saturating_mul(2)
                        .min(MAXIMUM_RETRANSMIT_TIMEOUT_NS);
                    socket.retransmit_at_ns = now_ns.saturating_add(socket.retransmit_timeout_ns);
                    continue;
                }
                match socket.state {
                    TcpState::SynSent => {
                        if socket.outstanding.is_empty() {
                            socket.start_outstanding(TCP_FLAG_SYN, &[], now_ns, transmissions)?;
                        }
                    }
                    TcpState::Established | TcpState::CloseWait => {
                        let send_window = usize::from(socket.peer_window)
                            .min(socket.congestion_window_bytes);
                        let mut available_flight =
                            send_window.saturating_sub(socket.outstanding_payload_bytes);
                        while !socket.send_buffer.is_empty()
                            && available_flight != 0
                            && socket.outstanding.len() < MAX_TCP_OUTSTANDING_SEGMENTS
                        {
                            let length = socket
                                .send_buffer
                                .len()
                                .min(available_flight)
                                .min(maximum_segment_size);
                            let mut payload = Vec::new();
                            payload
                                .try_reserve_exact(length)
                                .map_err(|_| Error::NoMem)?;
                            payload.extend(socket.send_buffer.iter().take(length).copied());
                            socket.start_outstanding(
                                TCP_FLAG_ACK | TCP_FLAG_PSH,
                                &payload,
                                now_ns,
                                transmissions,
                            )?;
                            for _ in 0..length {
                                let _ = socket.send_buffer.pop_front();
                            }
                            removed_send_bytes = removed_send_bytes.saturating_add(length);
                            available_flight -= length;
                        }
                        if socket.close_requested
                            && socket.send_buffer.is_empty()
                            && socket.outstanding.is_empty()
                        {
                            let flags = TCP_FLAG_ACK | TCP_FLAG_FIN;
                            socket.start_outstanding(flags, &[], now_ns, transmissions)?;
                            socket.state = if socket.state == TcpState::CloseWait {
                                TcpState::LastAck
                            } else {
                                TcpState::FinWait1
                            };
                        }
                    }
                    TcpState::Listen
                    | TcpState::SynReceived
                    | TcpState::FinWait1
                    | TcpState::FinWait2
                    | TcpState::Closing
                    | TcpState::TimeWait
                    | TcpState::LastAck
                    | TcpState::Closed => {}
                }
            }
            Ok(())
        })();
        self.queued_send_bytes = self.queued_send_bytes.saturating_sub(removed_send_bytes);
        for (handle, flow_key, listener_port) in removed_indexes.drain(..) {
            if let Some(key) = flow_key {
                self.remove_flow(key, handle);
            }
            if let Some(port) = listener_port {
                self.remove_listener(port, handle);
            }
        }
        removed_indexes.clear();
        self.removed_indexes = removed_indexes;
        poll_result?;
        Ok(changed)
    }

    pub(crate) fn receive_segment(
        &mut self,
        source_address: Ipv4Address,
        destination_address: Ipv4Address,
        segment: &TcpSegment<'_>,
        now_ns: u64,
        transmissions: &mut Vec<TcpTransmission>,
    ) -> Result<bool, Error> {
        transmissions.try_reserve(1).map_err(|_| Error::NoMem)?;
        let flow_key = TcpFlowKey {
            local_address: destination_address,
            remote_address: source_address,
            local_port: segment.destination_port,
            remote_port: segment.source_port,
        };
        if let Some(handle) = self.flow_handle(flow_key) {
            let index = handle.index;
            let mut accepted_listener = None;
            let changed = {
                let socket = self.slots[index]
                    .socket
                    .as_mut()
                    .ok_or(Error::Proto)?;
                if segment.flags & TCP_FLAG_RST != 0 {
                    let valid_reset = match socket.state {
                        TcpState::SynSent => {
                            segment.flags & TCP_FLAG_ACK != 0
                                && segment.acknowledgment_number == socket.send_next
                        }
                        TcpState::SynReceived
                        | TcpState::Established
                        | TcpState::FinWait1
                        | TcpState::FinWait2
                        | TcpState::CloseWait
                        | TcpState::Closing
                        | TcpState::TimeWait
                        | TcpState::LastAck => {
                            socket.receive_next == Some(segment.sequence_number)
                        }
                        TcpState::Closed | TcpState::Listen => false,
                    };
                    if !valid_reset {
                        return Ok(false);
                    }
                    socket.state = TcpState::Closed;
                    if socket.parent_listener.is_some() {
                        socket.orphaned = true;
                        socket.close_deadline_ns = Some(now_ns);
                    }
                    socket.outstanding.clear();
                    socket.outstanding_payload_bytes = 0;
                    return Ok(true);
                }
                match socket.state {
                    TcpState::SynSent => {
                        if segment.flags & (TCP_FLAG_SYN | TCP_FLAG_ACK)
                            != (TCP_FLAG_SYN | TCP_FLAG_ACK)
                            || segment.acknowledgment_number != socket.send_next
                        {
                            false
                        } else {
                            socket.acknowledge_outstanding(segment.acknowledgment_number, now_ns);
                            socket.receive_next = Some(segment.sequence_number.wrapping_add(1));
                            socket.peer_window = segment.window;
                            socket.state = TcpState::Established;
                            socket.retransmit_timeout_ns = INITIAL_RETRANSMIT_TIMEOUT_NS;
                            socket.retransmissions = 0;
                            transmissions.push(socket.transmission(
                                socket.send_next,
                                TCP_FLAG_ACK,
                                &[],
                            )?);
                            true
                        }
                    }
                    TcpState::SynReceived => {
                        if segment.flags & TCP_FLAG_SYN != 0 {
                            if let Some(outstanding) = socket
                                .outstanding
                                .iter()
                                .find(|outstanding| outstanding.flags & TCP_FLAG_SYN != 0)
                            {
                                transmissions.push(socket.transmission(
                                    outstanding.sequence_number,
                                    outstanding.flags,
                                    &outstanding.payload,
                                )?);
                            } else {
                                transmissions.push(socket.transmission(
                                    socket.send_next,
                                    TCP_FLAG_ACK,
                                    &[],
                                )?);
                            }
                            false
                        } else if segment.flags & TCP_FLAG_ACK == 0
                            || socket.receive_next != Some(segment.sequence_number)
                            || segment.acknowledgment_number != socket.send_next
                        {
                            false
                        } else {
                            socket.acknowledge_outstanding(segment.acknowledgment_number, now_ns);
                            socket.peer_window = segment.window;
                            socket.state = TcpState::Established;
                            accepted_listener = socket.parent_listener;
                            let _segment_changed = Self::receive_connected_segment(
                                socket,
                                segment,
                                now_ns,
                                transmissions,
                            )?;
                            true
                        }
                    }
                    TcpState::Established
                    | TcpState::FinWait1
                    | TcpState::FinWait2
                    | TcpState::CloseWait
                    | TcpState::Closing
                    | TcpState::TimeWait
                    | TcpState::LastAck => {
                        Self::receive_connected_segment(socket, segment, now_ns, transmissions)?
                    }
                    TcpState::Closed | TcpState::Listen => false,
                }
            };
            if let Some(listener) = accepted_listener {
                self.enqueue_accepted(listener, handle)?;
            }
            return Ok(changed);
        }

        if segment.flags & TCP_FLAG_SYN == 0 || segment.flags & TCP_FLAG_ACK != 0 {
            return Ok(false);
        }
        let Some(listener) = self.listener_handle(destination_address, segment.destination_port) else {
            return Ok(false);
        };
        let listener_index = listener.index;
        if self.socket(listener).is_none_or(|socket| socket.state != TcpState::Listen) {
            return Ok(false);
        }
        let backlog = self.slots[listener_index]
            .socket
            .as_ref()
            .map_or(0, |socket| socket.backlog);
        let pending_children = self
            .slots
            .iter()
            .filter_map(|slot| slot.socket.as_ref())
            .filter(|socket| {
                socket.parent_listener == Some(listener) && socket.state != TcpState::Closed
            })
            .count();
        if pending_children >= backlog {
            return Ok(false);
        }
        let initial_sequence =
            crate::kernel::arch::CpuArchitecture::random_u32().ok_or(Error::NoSys)?;
        let Ok((child_index, child_generation)) = self.allocate_slot() else {
            return Ok(false);
        };
        let mut child = TcpSocket::new(
            destination_address,
            source_address,
            segment.destination_port,
            segment.source_port,
            initial_sequence,
            TcpState::SynReceived,
            self.maximum_segment_size,
        )?;
        child.parent_listener = Some(listener);
        child.receive_next = Some(segment.sequence_number.wrapping_add(1));
        child.peer_window = segment.window;
        let transmission_count = transmissions.len();
        child.start_outstanding(
            TCP_FLAG_SYN | TCP_FLAG_ACK,
            &[],
            now_ns,
            transmissions,
        )?;
        self.slots[child_index].socket = Some(child);
        let child_handle = TcpSocketHandle {
            index: child_index,
            generation: child_generation,
        };
        if let Err(error) = self.add_flow(
            TcpFlowKey {
                local_address: destination_address,
                remote_address: source_address,
                local_port: segment.destination_port,
                remote_port: segment.source_port,
            },
            child_handle,
        ) {
            self.slots[child_index].socket = None;
            transmissions.truncate(transmission_count);
            return Err(error);
        }
        Ok(true)
    }

    fn enqueue_accepted(
        &mut self,
        listener: TcpSocketHandle,
        child: TcpSocketHandle,
    ) -> Result<(), Error> {
        let listener_index = self.slot_index(listener).map_err(|_| Error::BadFd)?;
        let (before, current_and_after) = self.slots.split_at_mut(listener_index);
        let (listener_slot, after) = current_and_after.split_first_mut().ok_or(Error::BadFd)?;
        if listener_slot.generation != listener.generation {
            return Err(Error::BadFd);
        }
        let socket = listener_slot.socket.as_mut().ok_or(Error::BadFd)?;
        if socket.state != TcpState::Listen {
            return Err(Error::Inval);
        }
        socket.accept_queue.retain(|handle| {
            let child_slot = if handle.index < listener_index {
                before.get(handle.index)
            } else if handle.index > listener_index {
                after.get(handle.index - listener_index - 1)
            } else {
                None
            };
            child_slot.is_some_and(|slot| {
                slot.generation == handle.generation
                    && slot.socket.as_ref().is_some_and(|child| {
                        child.state != TcpState::Closed
                            && child.parent_listener == Some(listener)
                    })
            })
        });
        if socket.accept_queue.len() >= socket.backlog {
            return Err(Error::NoBufs);
        }
        socket.accept_queue.push_back(child);
        Ok(())
    }

    fn receive_connected_segment(
        socket: &mut TcpSocket,
        segment: &TcpSegment<'_>,
        now_ns: u64,
        transmissions: &mut Vec<TcpTransmission>,
    ) -> Result<bool, Error> {
        if segment.flags & TCP_FLAG_SYN != 0 {
            transmissions.push(socket.transmission(socket.send_next, TCP_FLAG_ACK, &[])?);
            return Ok(false);
        }
        if segment.flags & TCP_FLAG_ACK == 0 {
            return Ok(false);
        }
        if !Self::sequence_is_acceptable(socket, segment) {
            let duplicate_fin_end = segment
                .sequence_number
                .wrapping_add(segment.payload.len() as u32)
                .wrapping_add(1);
            if socket.state == TcpState::TimeWait
                && segment.flags & TCP_FLAG_FIN != 0
                && socket.receive_next == Some(duplicate_fin_end)
            {
                socket.close_deadline_ns = Some(now_ns.saturating_add(TCP_TIME_WAIT_NS));
            }
            transmissions.push(socket.transmission(socket.send_next, TCP_FLAG_ACK, &[])?);
            return Ok(false);
        }
        let acceptable_acknowledgment = if segment.flags & TCP_FLAG_ACK != 0 {
            let acknowledgment_distance =
                segment.acknowledgment_number.wrapping_sub(socket.send_unacknowledged) as i32;
            let flight_distance = socket.send_next.wrapping_sub(socket.send_unacknowledged) as i32;
            if acknowledgment_distance > flight_distance {
                transmissions.push(socket.transmission(socket.send_next, TCP_FLAG_ACK, &[])?);
                return Ok(false);
            }
            acknowledgment_distance >= 0
        } else {
            false
        };
        let mut changed = false;
        if acceptable_acknowledgment {
            socket.peer_window = segment.window;
            changed |= socket.acknowledge_outstanding(segment.acknowledgment_number, now_ns);
        }
        let mut send_ack = false;
        let fin = segment.flags & TCP_FLAG_FIN != 0;
        if !segment.payload.is_empty() || fin {
            send_ack = true;
            changed |= Self::receive_sequence(
                socket,
                segment.sequence_number,
                segment.payload,
                fin,
                now_ns,
            )?;
        }
        if send_ack {
            transmissions.push(socket.transmission(
                socket.send_next,
                TCP_FLAG_ACK,
                &[],
            )?);
        }
        Ok(changed)
    }

    fn sequence_is_acceptable(socket: &TcpSocket, segment: &TcpSegment<'_>) -> bool {
        if segment.flags & TCP_FLAG_SYN != 0 {
            return false;
        }
        let Some(expected_sequence) = socket.receive_next else {
            return false;
        };
        let occupied = socket
            .receive_buffer
            .len()
            .saturating_add(socket.out_of_order_bytes);
        let receive_window = MAX_TCP_RECEIVE_WINDOW.saturating_sub(occupied);
        let segment_length = segment
            .payload
            .len()
            .saturating_add(usize::from(segment.flags & TCP_FLAG_FIN != 0));
        let sequence_delta = segment.sequence_number.wrapping_sub(expected_sequence) as i32;
        if receive_window == 0 {
            return segment_length == 0 && sequence_delta == 0;
        }
        if segment_length == 0 {
            return sequence_delta >= 0 && (sequence_delta as usize) < receive_window;
        }
        let start = i64::from(sequence_delta);
        let end = start.saturating_add(segment_length as i64);
        end > 0 && start < receive_window as i64
    }

    fn receive_sequence(
        socket: &mut TcpSocket,
        sequence_number: u32,
        payload: &[u8],
        fin: bool,
        now_ns: u64,
    ) -> Result<bool, Error> {
        let Some(expected_sequence) = socket.receive_next else {
            return Ok(false);
        };
        let sequence_delta = sequence_number.wrapping_sub(expected_sequence) as i32;
        let (start_offset, payload) = if sequence_delta < 0 {
            let bytes_before_window = (-i64::from(sequence_delta)) as usize;
            if bytes_before_window > MAX_TCP_RECEIVE_WINDOW || bytes_before_window > payload.len() {
                return Ok(false);
            }
            (0, &payload[bytes_before_window..])
        } else {
            (sequence_delta as usize, payload)
        };

        let occupied = socket
            .receive_buffer
            .len()
            .saturating_add(socket.out_of_order_bytes);
        let available = MAX_TCP_RECEIVE_WINDOW.saturating_sub(occupied);
        if start_offset >= available {
            return Ok(false);
        }
        let mut accepted_length = payload.len().min(available - start_offset);

        if start_offset == 0 {
            for queued in &socket.out_of_order {
                let queued_offset = queued.sequence_number.wrapping_sub(expected_sequence) as i32;
                if queued_offset > 0 {
                    accepted_length = accepted_length.min(queued_offset as usize);
                }
            }

            if accepted_length != 0 {
                socket
                    .receive_buffer
                    .try_reserve(accepted_length)
                    .map_err(|_| Error::NoMem)?;
                socket
                    .receive_buffer
                    .extend(payload[..accepted_length].iter().copied());
                socket.receive_next = Some(
                    expected_sequence.wrapping_add(accepted_length as u32),
                );
            }
            let mut changed = accepted_length != 0;
            if fin && accepted_length == payload.len() && accepted_length < available {
                Self::accept_fin(socket, now_ns);
                return Ok(true);
            }
            changed |= Self::drain_out_of_order(socket, now_ns)?;
            return Ok(changed);
        }

        let accepted_fin = fin
            && accepted_length == payload.len()
            && start_offset + accepted_length < available;
        if accepted_length == 0 && !accepted_fin {
            return Ok(false);
        }
        let new_end = start_offset + accepted_length + usize::from(accepted_fin);
        for queued in &socket.out_of_order {
            let queued_delta = queued.sequence_number.wrapping_sub(expected_sequence) as i32;
            if queued_delta < 0 {
                continue;
            }
            let queued_start = queued_delta as usize;
            let existing_end = queued_start + queued.payload.len() + usize::from(queued.fin);
            if start_offset < existing_end && queued_start < new_end {
                return Ok(false);
            }
        }
        if socket.out_of_order.len() >= MAX_TCP_OUT_OF_ORDER_SEGMENTS {
            return Ok(false);
        }
        let mut queued_payload = Vec::new();
        queued_payload
            .try_reserve_exact(accepted_length)
            .map_err(|_| Error::NoMem)?;
        queued_payload.extend_from_slice(&payload[..accepted_length]);
        socket
            .out_of_order
            .try_reserve(1)
            .map_err(|_| Error::NoMem)?;
        socket.out_of_order.push(ReassemblySegment {
            sequence_number: expected_sequence.wrapping_add(start_offset as u32),
            payload: queued_payload,
            fin: accepted_fin,
        });
        socket.out_of_order.sort_by_key(|queued| {
            queued.sequence_number.wrapping_sub(expected_sequence)
        });
        socket.out_of_order_bytes += accepted_length;
        Ok(true)
    }

    fn drain_out_of_order(socket: &mut TcpSocket, now_ns: u64) -> Result<bool, Error> {
        let mut changed = false;
        loop {
            let Some(expected_sequence) = socket.receive_next else {
                break;
            };
            let Some(index) = socket
                .out_of_order
                .iter()
                .position(|queued| queued.sequence_number == expected_sequence)
            else {
                break;
            };
            let payload_length = socket.out_of_order[index].payload.len();
            socket
                .receive_buffer
                .try_reserve(payload_length)
                .map_err(|_| Error::NoMem)?;
            let queued = socket.out_of_order.remove(index);
            socket
                .receive_buffer
                .extend(queued.payload.iter().copied());
            socket.out_of_order_bytes = socket
                .out_of_order_bytes
                .saturating_sub(payload_length);
            socket.receive_next = Some(expected_sequence.wrapping_add(payload_length as u32));
            changed = true;
            if queued.fin {
                Self::accept_fin(socket, now_ns);
                return Ok(true);
            }
        }
        Ok(changed)
    }

    fn accept_fin(socket: &mut TcpSocket, now_ns: u64) {
        socket.receive_next = socket.receive_next.map(|sequence| sequence.wrapping_add(1));
        socket.state = match socket.state {
            TcpState::Established => TcpState::CloseWait,
            TcpState::FinWait1 => TcpState::Closing,
            TcpState::FinWait2 => {
                socket.close_deadline_ns = Some(now_ns.saturating_add(TCP_TIME_WAIT_NS));
                TcpState::TimeWait
            }
            TcpState::Closing => {
                socket.close_deadline_ns = Some(now_ns.saturating_add(TCP_TIME_WAIT_NS));
                TcpState::TimeWait
            }
            state => state,
        };
        socket.out_of_order.clear();
        socket.out_of_order_bytes = 0;
    }

    fn slot_index(&self, handle: TcpSocketHandle) -> Result<usize, Error> {
        let slot = self.slots.get(handle.index).ok_or(Error::BadFd)?;
        if slot.generation != handle.generation || slot.socket.is_none() {
            return Err(Error::BadFd);
        }
        Ok(handle.index)
    }

    fn socket(&self, handle: TcpSocketHandle) -> Option<&TcpSocket> {
        let slot = self.slots.get(handle.index)?;
        (slot.generation == handle.generation)
            .then_some(slot.socket.as_ref())
            .flatten()
    }

    fn socket_mut(&mut self, handle: TcpSocketHandle) -> Option<&mut TcpSocket> {
        let slot = self.slots.get_mut(handle.index)?;
        if slot.generation != handle.generation {
            return None;
        }
        slot.socket.as_mut()
    }
}

impl TcpSocket {
    fn new(
        local_address: Ipv4Address,
        remote_address: Ipv4Address,
        local_port: u16,
        remote_port: u16,
        initial_sequence: u32,
        state: TcpState,
        maximum_segment_size: usize,
    ) -> Result<Self, Error> {
        Ok(Self {
            state,
            local_address,
            remote_address,
            local_port,
            remote_port,
            send_unacknowledged: initial_sequence,
            send_next: initial_sequence,
            receive_next: None,
            peer_window: u16::MAX,
            send_buffer: VecDeque::new(),
            receive_buffer: VecDeque::new(),
            out_of_order: Vec::new(),
            out_of_order_bytes: 0,
            maximum_segment_size,
            congestion_window_bytes: maximum_segment_size,
            slow_start_threshold_bytes: MAX_TCP_RECEIVE_WINDOW,
            outstanding: VecDeque::new(),
            outstanding_payload_bytes: 0,
            retransmit_at_ns: 0,
            retransmit_timeout_ns: INITIAL_RETRANSMIT_TIMEOUT_NS,
            retransmissions: 0,
            close_requested: false,
            orphaned: false,
            flow_indexed: false,
            listener_indexed: false,
            close_deadline_ns: None,
            parent_listener: None,
            accept_queue: VecDeque::new(),
            backlog: 0,
        })
    }

    fn start_outstanding(
        &mut self,
        flags: u8,
        payload: &[u8],
        now_ns: u64,
        transmissions: &mut Vec<TcpTransmission>,
    ) -> Result<(), Error> {
        if self.outstanding.len() >= MAX_TCP_OUTSTANDING_SEGMENTS {
            return Err(Error::NoBufs);
        }
        let sequence_number = self.send_next;
        let mut saved_payload = Vec::new();
        saved_payload
            .try_reserve_exact(payload.len())
            .map_err(|_| Error::NoMem)?;
        saved_payload.extend_from_slice(payload);
        let sequence_length = payload.len() as u32
            + u32::from(flags & TCP_FLAG_SYN != 0)
            + u32::from(flags & TCP_FLAG_FIN != 0);
        let end_sequence_number = sequence_number.wrapping_add(sequence_length);
        let transmission = self.transmission(sequence_number, flags, payload)?;
        transmissions.try_reserve(1).map_err(|_| Error::NoMem)?;
        self.outstanding.try_reserve(1).map_err(|_| Error::NoMem)?;
        let was_empty = self.outstanding.is_empty();
        self.outstanding.push_back(OutstandingSegment {
            sequence_number,
            end_sequence_number,
            flags,
            payload: saved_payload,
        });
        self.outstanding_payload_bytes =
            self.outstanding_payload_bytes.saturating_add(payload.len());
        self.send_next = end_sequence_number;
        if was_empty {
            self.retransmit_timeout_ns = INITIAL_RETRANSMIT_TIMEOUT_NS;
            self.retransmit_at_ns = now_ns.saturating_add(self.retransmit_timeout_ns);
            self.retransmissions = 0;
        }
        transmissions.push(transmission);
        Ok(())
    }

    fn acknowledge_outstanding(&mut self, acknowledgment_number: u32, now_ns: u64) -> bool {
        if self.outstanding.is_empty() {
            return false;
        }
        let acknowledgment_distance =
            acknowledgment_number.wrapping_sub(self.send_unacknowledged) as i32;
        let flight_distance = self.send_next.wrapping_sub(self.send_unacknowledged) as i32;
        if acknowledgment_distance <= 0 || flight_distance < 0 || acknowledgment_distance > flight_distance {
            return false;
        }

        let acknowledgment_distance = acknowledgment_distance as u32;
        let mut remaining = acknowledgment_distance;
        for outstanding in &self.outstanding {
            let sequence_length = outstanding
                .end_sequence_number
                .wrapping_sub(outstanding.sequence_number);
            if remaining <= sequence_length {
                if remaining < sequence_length
                    && (outstanding.flags & (TCP_FLAG_SYN | TCP_FLAG_FIN) != 0
                        || remaining as usize > outstanding.payload.len())
                {
                    return false;
                }
                remaining = 0;
                break;
            }
            remaining -= sequence_length;
        }
        if remaining != 0 {
            return false;
        }

        let mut remaining = acknowledgment_distance;
        let mut acknowledged_payload = 0usize;
        let mut acknowledged_fin = false;
        while remaining != 0 {
            let Some(front) = self.outstanding.front() else {
                return false;
            };
            let sequence_length = front
                .end_sequence_number
                .wrapping_sub(front.sequence_number);
            if remaining < sequence_length {
                let acknowledged = remaining as usize;
                let Some(front) = self.outstanding.front_mut() else {
                    return false;
                };
                front.payload.drain(..acknowledged);
                front.sequence_number = acknowledgment_number;
                self.outstanding_payload_bytes = self
                    .outstanding_payload_bytes
                    .saturating_sub(acknowledged);
                acknowledged_payload += acknowledged;
                remaining = 0;
            } else {
                let Some(outstanding) = self.outstanding.pop_front() else {
                    return false;
                };
                remaining -= sequence_length;
                acknowledged_payload += outstanding.payload.len();
                self.outstanding_payload_bytes = self
                    .outstanding_payload_bytes
                    .saturating_sub(outstanding.payload.len());
                acknowledged_fin |= outstanding.flags & TCP_FLAG_FIN != 0;
            }
        }

        self.send_unacknowledged = acknowledgment_number;
        self.retransmissions = 0;
        self.retransmit_timeout_ns = INITIAL_RETRANSMIT_TIMEOUT_NS;
        if !self.outstanding.is_empty() {
            self.retransmit_at_ns = now_ns.saturating_add(self.retransmit_timeout_ns);
        }
        if acknowledged_payload != 0 {
            let increase = if self.congestion_window_bytes < self.slow_start_threshold_bytes {
                acknowledged_payload
            } else {
                self.maximum_segment_size
                    .saturating_mul(acknowledged_payload)
                    .checked_div(self.congestion_window_bytes.max(1))
                    .unwrap_or(1)
                    .max(1)
            };
            self.congestion_window_bytes = self
                .congestion_window_bytes
                .saturating_add(increase)
                .min(MAX_TCP_RECEIVE_WINDOW);
        }
        if acknowledged_fin {
            self.state = match self.state {
                TcpState::FinWait1 => TcpState::FinWait2,
                TcpState::Closing => {
                    self.close_deadline_ns = Some(now_ns.saturating_add(TCP_TIME_WAIT_NS));
                    TcpState::TimeWait
                }
                TcpState::LastAck => TcpState::Closed,
                state => state,
            };
        }
        true
    }

    fn on_retransmission_timeout(&mut self) {
        self.slow_start_threshold_bytes = (self.congestion_window_bytes / 2)
            .max(self.maximum_segment_size.saturating_mul(2));
        self.congestion_window_bytes = self.maximum_segment_size;
    }

    fn transmission(
        &self,
        sequence_number: u32,
        flags: u8,
        payload: &[u8],
    ) -> Result<TcpTransmission, Error> {
        let mut copied_payload = Vec::new();
        copied_payload
            .try_reserve_exact(payload.len())
            .map_err(|_| Error::NoMem)?;
        copied_payload.extend_from_slice(payload);
        Ok(TcpTransmission {
            source_address: self.local_address,
            destination_address: self.remote_address,
            source_port: self.local_port,
            destination_port: self.remote_port,
            sequence_number,
            acknowledgment_number: self.receive_next.unwrap_or(0),
            flags,
            window: (MAX_TCP_RECEIVE_WINDOW.saturating_sub(
                self.receive_buffer
                    .len()
                    .saturating_add(self.out_of_order_bytes),
            )) as u16,
            payload: copied_payload,
        })
    }
}

#[cfg(test)]
#[path = "../../../tests/kernel/net/tcp/connection.rs"]
mod tests;