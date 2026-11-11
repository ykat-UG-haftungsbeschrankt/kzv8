/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::collections::VecDeque;
use alloc::vec::Vec;

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
    LastAck,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TcpSocketHandle {
    index: usize,
    generation: u32,
}

struct OutstandingSegment {
    sequence_number: u32,
    end_sequence_number: u32,
    flags: u8,
    payload: Vec<u8>,
}

struct ReassemblySegment {
    sequence_number: u32,
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
    pub(crate) payload: Vec<u8>,
}

pub(crate) struct TcpSocketTable {
    slots: Vec<SocketSlot>,
    maximum_segment_size: usize,
    queued_send_bytes: usize,
}

impl TcpSocketTable {
    pub(crate) fn new(interface_mtu: usize) -> Option<Self> {
        let maximum_segment_size = interface_mtu
            .saturating_sub(20 + TCP_HEADER_LENGTH)
            .min(usize::from(u16::MAX));
        (maximum_segment_size != 0).then_some(Self {
            slots: Vec::new(),
            maximum_segment_size,
            queued_send_bytes: 0,
        })
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
        Ok(TcpSocketHandle { index, generation })
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
        if self.slots.iter().any(|slot| {
            slot.socket.as_ref().is_some_and(|socket| {
                socket.state == TcpState::Listen
                    && socket.local_port == local_port
                    && (socket.local_address == local_address
                        || socket.local_address == Ipv4Address([0; 4])
                        || local_address == Ipv4Address([0; 4]))
            })
        }) {
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
        Ok(TcpSocketHandle { index, generation })
    }

    pub(crate) fn accept(
        &mut self,
        listener: TcpSocketHandle,
    ) -> Result<Option<TcpSocketHandle>, Error> {
        let socket = self.socket_mut(listener).ok_or(Error::BadFd)?;
        if socket.state != TcpState::Listen {
            return Err(Error::Inval);
        }
        let Some(child) = socket.accept_queue.pop_front() else {
            return Ok(None);
        };
        if let Some(socket) = self.socket_mut(child) {
            socket.parent_listener = None;
        }
        Ok(Some(child))
    }

    pub(crate) fn state(&self, handle: TcpSocketHandle) -> Option<TcpState> {
        self.socket(handle).map(|socket| socket.state)
    }

    pub(crate) fn readable(&self, handle: TcpSocketHandle) -> bool {
        let Some(socket) = self.socket(handle) else {
            return false;
        };
        match socket.state {
            TcpState::Listen => !socket.accept_queue.is_empty(),
            TcpState::CloseWait | TcpState::Closed => true,
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
        let socket = self.socket_mut(handle).ok_or(Error::BadFd)?;
        socket.orphaned = true;
        socket.close_deadline_ns = Some(now_ns.saturating_add(TCP_CLOSE_TIMEOUT_NS));
        Self::request_close(socket);
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
        let buffered_send_bytes = {
            let Some(slot) = self.slots.get_mut(handle.index) else {
                return false;
            };
            if slot.generation != handle.generation || slot.socket.is_none() {
                return false;
            }
            let buffered_send_bytes = slot
                .socket
                .as_ref()
                .map_or(0, |socket| socket.send_buffer.len());
            slot.socket = None;
            buffered_send_bytes
        };
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
        let mut changed = false;
        let mut removed_send_bytes = 0usize;
        let poll_result = (|| -> Result<(), Error> {
            for slot in &mut self.slots {
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
                if socket.state == TcpState::Closed {
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
                    | TcpState::LastAck
                    | TcpState::Closed => {}
                }
            }
            Ok(())
        })();
        self.queued_send_bytes = self.queued_send_bytes.saturating_sub(removed_send_bytes);
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
        let connection_index = self.slots.iter().position(|slot| {
            slot.socket.as_ref().is_some_and(|socket| {
                socket.state != TcpState::Listen
                    && socket.state != TcpState::Closed
                    && socket.local_address == destination_address
                    && socket.remote_address == source_address
                    && socket.local_port == segment.destination_port
                    && socket.remote_port == segment.source_port
            })
        });
        if let Some(index) = connection_index {
            let handle = TcpSocketHandle {
                index,
                generation: self.slots[index].generation,
            };
            let mut accepted_listener = None;
            let changed = {
                let socket = self.slots[index]
                    .socket
                    .as_mut()
                    .ok_or(Error::Proto)?;
                if segment.flags & TCP_FLAG_RST != 0 {
                    socket.state = TcpState::Closed;
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
                        if segment.flags & TCP_FLAG_ACK == 0
                            || segment.acknowledgment_number != socket.send_next
                        {
                            false
                        } else {
                            socket.acknowledge_outstanding(segment.acknowledgment_number, now_ns);
                            socket.peer_window = segment.window;
                            socket.state = TcpState::Established;
                            accepted_listener = socket.parent_listener;
                            true
                        }
                    }
                    TcpState::Established
                    | TcpState::FinWait1
                    | TcpState::FinWait2
                    | TcpState::CloseWait
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
        let Some(listener_index) = self.slots.iter().position(|slot| {
            slot.socket.as_ref().is_some_and(|socket| {
                socket.state == TcpState::Listen
                    && socket.local_port == segment.destination_port
                    && (socket.local_address == destination_address
                        || socket.local_address == Ipv4Address([0; 4]))
            })
        }) else {
            return Ok(false);
        };
        let listener = TcpSocketHandle {
            index: listener_index,
            generation: self.slots[listener_index].generation,
        };
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
        child.start_outstanding(
            TCP_FLAG_SYN | TCP_FLAG_ACK,
            &[],
            now_ns,
            transmissions,
        )?;
        self.slots[child_index].socket = Some(child);
        Ok(true)
    }

    fn enqueue_accepted(
        &mut self,
        listener: TcpSocketHandle,
        child: TcpSocketHandle,
    ) -> Result<(), Error> {
        let socket = self.socket_mut(listener).ok_or(Error::BadFd)?;
        if socket.state != TcpState::Listen || socket.accept_queue.len() >= socket.backlog {
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
        let mut changed = false;
        if segment.flags & TCP_FLAG_ACK != 0 {
            socket.peer_window = segment.window;
            changed |= socket.acknowledge_outstanding(segment.acknowledgment_number, now_ns);
        }
        let mut send_ack = false;
        let fin = segment.flags & TCP_FLAG_FIN != 0;
        if !segment.payload.is_empty() || fin {
            send_ack = true;
            changed |= Self::receive_sequence(socket, segment.sequence_number, segment.payload, fin)?;
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

    fn receive_sequence(
        socket: &mut TcpSocket,
        sequence_number: u32,
        payload: &[u8],
        fin: bool,
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
                Self::accept_fin(socket);
                return Ok(true);
            }
            changed |= Self::drain_out_of_order(socket)?;
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

    fn drain_out_of_order(socket: &mut TcpSocket) -> Result<bool, Error> {
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
                Self::accept_fin(socket);
                return Ok(true);
            }
        }
        Ok(changed)
    }

    fn accept_fin(socket: &mut TcpSocket) {
        socket.receive_next = socket.receive_next.map(|sequence| sequence.wrapping_add(1));
        socket.state = match socket.state {
            TcpState::Established => TcpState::CloseWait,
            TcpState::FinWait1 | TcpState::FinWait2 => TcpState::Closed,
            state => state,
        };
        socket.out_of_order.clear();
        socket.out_of_order_bytes = 0;
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
mod tests {
    use alloc::vec::Vec;

    use super::{
        TcpSocketTable, TcpState, TcpTransmission, MAX_TCP_SEND_BUFFER_PER_SOCKET,
        MAX_TCP_SEND_BUFFER_TOTAL, MAX_TCP_SOCKETS, MAX_TCP_RECEIVE_WINDOW,
    };
    use crate::kernel::net::tcp::segment::{
        write_segment, TcpSegment, TCP_FLAG_ACK, TCP_FLAG_PSH, TCP_FLAG_SYN, TCP_HEADER_LENGTH,
    };
    use crate::kernel::net::wire::Ipv4Address;

    fn encode_transmission(transmission: &TcpTransmission) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.resize(TCP_HEADER_LENGTH + transmission.payload.len(), 0);
        let length = write_segment(
            &mut bytes,
            transmission.source_address,
            transmission.destination_address,
            transmission.source_port,
            transmission.destination_port,
            transmission.sequence_number,
            transmission.acknowledgment_number,
            transmission.flags,
            transmission.window,
            &transmission.payload,
        )
        .unwrap();
        bytes.truncate(length);
        bytes
    }

    #[test]
    fn client_enters_established_after_syn_ack() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 0x1020_3040)
            .unwrap();
        let mut transmissions = Vec::new();
        sockets.poll(1, &mut transmissions).unwrap();
        assert_eq!(transmissions.len(), 1);
        assert_eq!(transmissions[0].flags, TCP_FLAG_SYN);
        let initial_sequence = transmissions[0].sequence_number;
        transmissions.clear();

        let mut response = [0; 20];
        let response_length = write_segment(
            &mut response,
            remote,
            local,
            443,
            49152,
            1234,
            initial_sequence.wrapping_add(1),
            TCP_FLAG_SYN | TCP_FLAG_ACK,
            4096,
            &[],
        )
        .unwrap();
        let response = TcpSegment::parse(&response[..response_length], remote, local).unwrap();
        assert!(sockets
            .receive_segment(remote, local, &response, 2, &mut transmissions)
            .unwrap());
        assert_eq!(sockets.state(handle), Some(TcpState::Established));
        assert_eq!(transmissions.len(), 1);
        assert_eq!(transmissions[0].flags, TCP_FLAG_ACK);
    }

    #[test]
    fn stale_socket_handle_is_rejected() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 0x1020_3040)
            .unwrap();
        assert!(sockets.remove(handle));
        assert_eq!(sockets.state(handle), None);
    }

    #[test]
    fn socket_table_grows_beyond_initial_capacity() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        for index in 0..MAX_TCP_SOCKETS {
            sockets
                .connect_with_initial_sequence(
                    local,
                    remote,
                    49152 + index as u16,
                    443,
                    index as u32,
                )
                .unwrap();
        }
        assert_eq!(sockets.slots.len(), MAX_TCP_SOCKETS);
        assert!(matches!(
            sockets.connect_with_initial_sequence(local, remote, 49408, 443, 1),
            Err(crate::kernel::error::Error::NoBufs)
        ));
    }

    #[test]
    fn send_buffer_limits_accept_partial_data_then_apply_backpressure() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 0x1020_3040)
            .unwrap();
        sockets.socket_mut(handle).unwrap().state = TcpState::Established;
        let mut payload = Vec::new();
        payload.resize(MAX_TCP_SEND_BUFFER_PER_SOCKET + 1, 0x5a);

        assert_eq!(
            sockets.send(handle, &payload).unwrap(),
            MAX_TCP_SEND_BUFFER_PER_SOCKET
        );
        assert_eq!(sockets.queued_send_bytes, MAX_TCP_SEND_BUFFER_PER_SOCKET);
        assert!(matches!(
            sockets.send(handle, b"next"),
            Err(crate::kernel::error::Error::NoBufs)
        ));
        assert!(!sockets.writable(handle));
        assert!(sockets.remove(handle));
        assert_eq!(sockets.queued_send_bytes, 0);

        for index in 0..(MAX_TCP_SEND_BUFFER_TOTAL / MAX_TCP_SEND_BUFFER_PER_SOCKET) {
            let handle = sockets
                .connect_with_initial_sequence(
                    local,
                    remote,
                    49152 + index as u16,
                    443,
                    index as u32,
                )
                .unwrap();
            sockets.socket_mut(handle).unwrap().state = TcpState::Established;
            assert_eq!(
                sockets
                    .send(handle, &payload[..MAX_TCP_SEND_BUFFER_PER_SOCKET])
                    .unwrap(),
                MAX_TCP_SEND_BUFFER_PER_SOCKET
            );
        }
        assert_eq!(sockets.queued_send_bytes, MAX_TCP_SEND_BUFFER_TOTAL);
        let overflow_handle = sockets
            .connect_with_initial_sequence(local, remote, 49216, 443, 1)
            .unwrap();
        sockets.socket_mut(overflow_handle).unwrap().state = TcpState::Established;
        assert!(matches!(
            sockets.send(overflow_handle, b"x"),
            Err(crate::kernel::error::Error::NoBufs)
        ));
        assert!(!sockets.writable(overflow_handle));
    }

    #[test]
    fn out_of_order_payload_is_reassembled_and_fin_waits_for_gap() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 0x1020_3040)
            .unwrap();
        let socket = sockets.socket_mut(handle).unwrap();
        socket.state = TcpState::Established;
        socket.receive_next = Some(1000);

        let future_segment = TcpSegment {
            source_port: 443,
            destination_port: 49152,
            sequence_number: 1005,
            acknowledgment_number: 0,
            flags: TCP_FLAG_ACK | crate::kernel::net::tcp::segment::TCP_FLAG_FIN,
            window: 4096,
            payload: b"world",
        };
        let mut transmissions = Vec::new();
        assert!(sockets
            .receive_segment(remote, local, &future_segment, 2, &mut transmissions)
            .unwrap());
        assert_eq!(transmissions[0].acknowledgment_number, 1000);
        assert_eq!(
            transmissions[0].window,
            (MAX_TCP_RECEIVE_WINDOW - b"world".len()) as u16
        );
        assert_eq!(
            sockets.socket(handle).unwrap().out_of_order_bytes,
            b"world".len()
        );

        let overlapping_segment = TcpSegment {
            source_port: 443,
            destination_port: 49152,
            sequence_number: 1007,
            acknowledgment_number: 0,
            flags: TCP_FLAG_ACK,
            window: 4096,
            payload: b"rl",
        };
        transmissions.clear();
        assert!(!sockets
            .receive_segment(remote, local, &overlapping_segment, 3, &mut transmissions)
            .unwrap());

        let in_order_segment = TcpSegment {
            source_port: 443,
            destination_port: 49152,
            sequence_number: 1000,
            acknowledgment_number: 0,
            flags: TCP_FLAG_ACK,
            window: 4096,
            payload: b"hello",
        };
        transmissions.clear();
        assert!(sockets
            .receive_segment(remote, local, &in_order_segment, 4, &mut transmissions)
            .unwrap());
        assert_eq!(sockets.state(handle), Some(TcpState::CloseWait));
        assert_eq!(transmissions[0].acknowledgment_number, 1011);

        let mut output = [0; 10];
        assert_eq!(sockets.receive(handle, &mut output).unwrap(), output.len());
        assert_eq!(&output, b"helloworld");
        assert_eq!(sockets.socket(handle).unwrap().out_of_order_bytes, 0);
    }

    #[test]
    fn congestion_window_grows_on_ack_and_reduces_on_timeout() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 0x1020_3040)
            .unwrap();
        let socket = sockets.socket_mut(handle).unwrap();
        socket.state = TcpState::Established;
        let initial_window = socket.congestion_window_bytes;
        let mut transmissions = Vec::new();
        socket
            .start_outstanding(
                TCP_FLAG_ACK | crate::kernel::net::tcp::segment::TCP_FLAG_PSH,
                &[0x5a; 1200],
                1,
                &mut transmissions,
            )
            .unwrap();
        let acknowledgment_number = socket.send_next;
        assert!(socket.acknowledge_outstanding(acknowledgment_number, 2));
        assert_eq!(socket.congestion_window_bytes, initial_window + 1200);

        let previous_window = socket.congestion_window_bytes;
        socket
            .start_outstanding(
                TCP_FLAG_ACK | crate::kernel::net::tcp::segment::TCP_FLAG_PSH,
                &[0xa5; 1000],
                2,
                &mut transmissions,
            )
            .unwrap();
        socket.on_retransmission_timeout();
        assert_eq!(socket.congestion_window_bytes, socket.maximum_segment_size);
        assert_eq!(
            socket.slow_start_threshold_bytes,
            (previous_window / 2).max(socket.maximum_segment_size * 2)
        );
    }

    #[test]
    fn poll_fills_but_does_not_exceed_congestion_window() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 0x1020_3040)
            .unwrap();
        let maximum_segment_size = sockets.maximum_segment_size;
        let socket = sockets.socket_mut(handle).unwrap();
        socket.state = TcpState::Established;
        socket.congestion_window_bytes = maximum_segment_size * 3;
        let mut payload = Vec::new();
        payload.resize(maximum_segment_size * 4, 0x3c);
        assert_eq!(sockets.send(handle, &payload).unwrap(), payload.len());

        let mut transmissions = Vec::new();
        sockets.poll(10, &mut transmissions).unwrap();
        assert_eq!(transmissions.len(), 3);
        let socket = sockets.socket(handle).unwrap();
        assert_eq!(socket.outstanding.len(), 3);
        assert_eq!(socket.outstanding_payload_bytes, maximum_segment_size * 3);
        assert_eq!(socket.send_buffer.len(), maximum_segment_size);

        let initial_sequence = 0x1020_3040;
        let partial_ack_number = initial_sequence + (maximum_segment_size * 3 / 2) as u32;
        let partial_ack = TcpSegment {
            source_port: 443,
            destination_port: 49152,
            sequence_number: 0,
            acknowledgment_number: partial_ack_number,
            flags: TCP_FLAG_ACK,
            window: u16::MAX,
            payload: &[],
        };
        assert!(sockets
            .receive_segment(remote, local, &partial_ack, 11, &mut Vec::new())
            .unwrap());
        let socket = sockets.socket(handle).unwrap();
        assert_eq!(socket.outstanding.len(), 2);
        assert_eq!(
            socket.outstanding_payload_bytes,
            maximum_segment_size * 3 - maximum_segment_size * 3 / 2
        );

        let cumulative_ack = TcpSegment {
            acknowledgment_number: initial_sequence
                .wrapping_add((maximum_segment_size * 3) as u32),
            ..partial_ack
        };
        assert!(sockets
            .receive_segment(remote, local, &cumulative_ack, 12, &mut Vec::new())
            .unwrap());
        let socket = sockets.socket(handle).unwrap();
        assert!(socket.outstanding.is_empty());
        assert_eq!(socket.outstanding_payload_bytes, 0);
    }

    #[test]
    fn listener_accepts_completed_handshake() {
        if crate::kernel::arch::CpuArchitecture::random_u32().is_none() {
            return;
        }
        let server = Ipv4Address([192, 0, 2, 1]);
        let client = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let listener = sockets.listen(server, 8080, 4, 1).unwrap();
        let mut transmissions = Vec::new();

        let mut syn_bytes = [0; 20];
        let syn_length = write_segment(
            &mut syn_bytes,
            client,
            server,
            49152,
            8080,
            100,
            0,
            TCP_FLAG_SYN,
            4096,
            &[],
        )
        .unwrap();
        let syn = TcpSegment::parse(&syn_bytes[..syn_length], client, server).unwrap();
        assert!(sockets
            .receive_segment(client, server, &syn, 2, &mut transmissions)
            .unwrap());
        assert_eq!(transmissions.len(), 1);
        assert_eq!(transmissions[0].flags, TCP_FLAG_SYN | TCP_FLAG_ACK);
        let server_sequence = transmissions[0].sequence_number;
        transmissions.clear();

        let mut ack_bytes = [0; 20];
        let ack_length = write_segment(
            &mut ack_bytes,
            client,
            server,
            49152,
            8080,
            101,
            server_sequence.wrapping_add(1),
            TCP_FLAG_ACK,
            4096,
            &[],
        )
        .unwrap();
        let ack = TcpSegment::parse(&ack_bytes[..ack_length], client, server).unwrap();
        assert!(sockets
            .receive_segment(client, server, &ack, 3, &mut transmissions)
            .unwrap());
        let accepted = sockets.accept(listener).unwrap().unwrap();
        assert_eq!(sockets.state(accepted), Some(TcpState::Established));
    }

    #[test]
    fn two_tcp_endpoints_exchange_payload_and_acknowledge_it() {
        if crate::kernel::arch::CpuArchitecture::random_u32().is_none() {
            return;
        }
        let client_address = Ipv4Address([192, 0, 2, 1]);
        let server_address = Ipv4Address([192, 0, 2, 2]);
        let mut client = TcpSocketTable::new(1500).unwrap();
        let mut server = TcpSocketTable::new(1500).unwrap();
        let listener = server.listen(server_address, 8080, 4, 1).unwrap();
        let client_socket = client
            .connect_with_initial_sequence(client_address, server_address, 49152, 8080, 1000)
            .unwrap();
        let mut client_transmissions = Vec::new();
        let mut server_transmissions = Vec::new();

        client.poll(1, &mut client_transmissions).unwrap();
        let syn_bytes = encode_transmission(&client_transmissions.remove(0));
        let syn = TcpSegment::parse(&syn_bytes, client_address, server_address).unwrap();
        assert!(server
            .receive_segment(client_address, server_address, &syn, 2, &mut server_transmissions)
            .unwrap());

        let syn_ack_bytes = encode_transmission(&server_transmissions.remove(0));
        let syn_ack = TcpSegment::parse(&syn_ack_bytes, server_address, client_address).unwrap();
        assert!(client
            .receive_segment(server_address, client_address, &syn_ack, 3, &mut client_transmissions)
            .unwrap());

        let ack_bytes = encode_transmission(&client_transmissions.remove(0));
        let ack = TcpSegment::parse(&ack_bytes, client_address, server_address).unwrap();
        assert!(server
            .receive_segment(client_address, server_address, &ack, 4, &mut server_transmissions)
            .unwrap());
        let accepted = server.accept(listener).unwrap().unwrap();
        assert_eq!(client.state(client_socket), Some(TcpState::Established));
        assert_eq!(server.state(accepted), Some(TcpState::Established));

        assert_eq!(client.send(client_socket, b"payload").unwrap(), 7);
        client.poll(5, &mut client_transmissions).unwrap();
        let data_bytes = encode_transmission(&client_transmissions.remove(0));
        let data = TcpSegment::parse(&data_bytes, client_address, server_address).unwrap();
        assert_eq!(data.flags & TCP_FLAG_PSH, TCP_FLAG_PSH);
        assert!(server
            .receive_segment(client_address, server_address, &data, 6, &mut server_transmissions)
            .unwrap());
        let mut output = [0; 7];
        assert_eq!(server.receive(accepted, &mut output).unwrap(), output.len());
        assert_eq!(&output, b"payload");

        let data_ack_bytes = encode_transmission(&server_transmissions.remove(0));
        let data_ack = TcpSegment::parse(&data_ack_bytes, server_address, client_address).unwrap();
        assert!(client
            .receive_segment(server_address, client_address, &data_ack, 7, &mut client_transmissions)
            .unwrap());
        assert!(client.writable(client_socket));
    }
}