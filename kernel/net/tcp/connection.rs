/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::kernel::error::Error;
use crate::kernel::net::tcp::segment::{
    TcpSegment, TCP_HEADER_LENGTH, TCP_FLAG_ACK, TCP_FLAG_FIN, TCP_FLAG_PSH, TCP_FLAG_RST,
    TCP_FLAG_SYN,
};
use crate::kernel::net::wire::Ipv4Address;

const MAX_TCP_RECEIVE_WINDOW: usize = u16::MAX as usize;
const MAX_TCP_SOCKETS: usize = 256;
const MAX_TCP_ACCEPT_BACKLOG: usize = 128;
const MAX_TCP_SEND_BUFFER_PER_SOCKET: usize = 64 * 1024;
const MAX_TCP_SEND_BUFFER_TOTAL: usize = 4 * 1024 * 1024;
const INITIAL_RETRANSMIT_TIMEOUT_NS: u64 = 1_000_000_000;
const MAXIMUM_RETRANSMIT_TIMEOUT_NS: u64 = 60_000_000_000;
const MAXIMUM_RETRANSMISSIONS: u8 = 5;
const TCP_CLOSE_TIMEOUT_NS: u64 = 120_000_000_000;

static NEXT_INITIAL_SEQUENCE: AtomicU32 = AtomicU32::new(0x7e57_0001);

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
    outstanding: Option<OutstandingSegment>,
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

    pub(crate) fn connect(
        &mut self,
        local_address: Ipv4Address,
        remote_address: Ipv4Address,
        local_port: u16,
        remote_port: u16,
        now_ns: u64,
    ) -> Result<TcpSocketHandle, Error> {
        if local_port == 0 || remote_port == 0 {
            return Err(Error::Inval);
        }
        let (index, generation) = self.allocate_slot()?;
        let sequence_number = NEXT_INITIAL_SEQUENCE
            .fetch_add(0x9e37_79b9, Ordering::Relaxed)
            .wrapping_add(now_ns as u32)
            .wrapping_add(u32::from(local_port));
        let socket = TcpSocket::new(
            local_address,
            remote_address,
            local_port,
            remote_port,
            sequence_number,
            TcpState::SynSent,
        )?;
        self.slots[index].socket = Some(socket);
        Ok(TcpSocketHandle { index, generation })
    }

    pub(crate) fn listen(
        &mut self,
        local_address: Ipv4Address,
        local_port: u16,
        backlog: usize,
        now_ns: u64,
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
        let sequence_number = NEXT_INITIAL_SEQUENCE
            .fetch_add(0x9e37_79b9, Ordering::Relaxed)
            .wrapping_add(now_ns as u32)
            .wrapping_add(u32::from(local_port));
        let mut socket = TcpSocket::new(
            local_address,
            Ipv4Address([0; 4]),
            local_port,
            0,
            sequence_number,
            TcpState::Listen,
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
        socket.send_buffer.len() < MAX_TCP_SEND_BUFFER_PER_SOCKET
            && self.queued_send_bytes < MAX_TCP_SEND_BUFFER_TOTAL
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
        let accepted = bytes
            .len()
            .min(MAX_TCP_SEND_BUFFER_PER_SOCKET.saturating_sub(socket.send_buffer.len()))
            .min(MAX_TCP_SEND_BUFFER_TOTAL.saturating_sub(self.queued_send_bytes));
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
                    socket.outstanding = None;
                }
                if socket.state == TcpState::Closed {
                    if socket.orphaned {
                        removed_send_bytes =
                            removed_send_bytes.saturating_add(socket.send_buffer.len());
                        slot.socket = None;
                    }
                    continue;
                }
                if socket.outstanding.is_some() {
                    if now_ns < socket.retransmit_at_ns {
                        continue;
                    }
                    if socket.retransmissions == MAXIMUM_RETRANSMISSIONS {
                        socket.state = TcpState::Closed;
                        socket.outstanding = None;
                        changed = true;
                        continue;
                    }
                    let outstanding = socket.outstanding.as_ref().ok_or(Error::Proto)?;
                    transmissions.push(socket.transmission(
                        outstanding.sequence_number,
                        outstanding.flags,
                        &outstanding.payload,
                    )?);
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
                        socket.start_outstanding(TCP_FLAG_SYN, &[], now_ns, transmissions)?;
                    }
                    TcpState::Established | TcpState::CloseWait => {
                        if !socket.send_buffer.is_empty() && socket.peer_window != 0 {
                            let length = socket
                                .send_buffer
                                .len()
                                .min(usize::from(socket.peer_window))
                                .min(maximum_segment_size);
                            let mut payload = Vec::new();
                            payload
                                .try_reserve_exact(length)
                                .map_err(|_| Error::NoMem)?;
                            for _ in 0..length {
                                if let Some(byte) = socket.send_buffer.pop_front() {
                                    payload.push(byte);
                                }
                            }
                            removed_send_bytes = removed_send_bytes.saturating_add(length);
                            socket.start_outstanding(
                                TCP_FLAG_ACK | TCP_FLAG_PSH,
                                &payload,
                                now_ns,
                                transmissions,
                            )?;
                        } else if socket.close_requested && socket.send_buffer.is_empty() {
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
                    socket.outstanding = None;
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
                            socket.acknowledge_outstanding(segment.acknowledgment_number);
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
                            socket.acknowledge_outstanding(segment.acknowledgment_number);
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
                        Self::receive_connected_segment(socket, segment, transmissions)?
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
        let Ok((child_index, child_generation)) = self.allocate_slot() else {
            return Ok(false);
        };
        let initial_sequence = NEXT_INITIAL_SEQUENCE
            .fetch_add(0x9e37_79b9, Ordering::Relaxed)
            .wrapping_add(now_ns as u32)
            .wrapping_add(u32::from(segment.destination_port));
        let mut child = TcpSocket::new(
            destination_address,
            source_address,
            segment.destination_port,
            segment.source_port,
            initial_sequence,
            TcpState::SynReceived,
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
        transmissions: &mut Vec<TcpTransmission>,
    ) -> Result<bool, Error> {
        let mut changed = false;
        if segment.flags & TCP_FLAG_ACK != 0 {
            socket.peer_window = segment.window;
            changed |= socket.acknowledge_outstanding(segment.acknowledgment_number);
        }
        let mut send_ack = false;
        let expected_sequence = socket.receive_next;
        if !segment.payload.is_empty() {
            send_ack = true;
            if expected_sequence == Some(segment.sequence_number) {
                let available = MAX_TCP_RECEIVE_WINDOW.saturating_sub(socket.receive_buffer.len());
                if segment.payload.len() <= available {
                    socket
                        .receive_buffer
                        .try_reserve(segment.payload.len())
                        .map_err(|_| Error::NoMem)?;
                    socket
                        .receive_buffer
                        .extend(segment.payload.iter().copied());
                    socket.receive_next = expected_sequence
                        .map(|value| value.wrapping_add(segment.payload.len() as u32));
                    changed = true;
                }
            }
        }
        if segment.flags & TCP_FLAG_FIN != 0 {
            let fin_sequence = segment
                .sequence_number
                .wrapping_add(segment.payload.len() as u32);
            if socket.receive_next == Some(fin_sequence) {
                socket.receive_next = Some(fin_sequence.wrapping_add(1));
                socket.state = match socket.state {
                    TcpState::Established => TcpState::CloseWait,
                    TcpState::FinWait1 | TcpState::FinWait2 => TcpState::Closed,
                    state => state,
                };
                send_ack = true;
                changed = true;
            }
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
            outstanding: None,
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
        self.outstanding = Some(OutstandingSegment {
            sequence_number,
            end_sequence_number,
            flags,
            payload: saved_payload,
        });
        self.send_next = end_sequence_number;
        self.retransmit_timeout_ns = INITIAL_RETRANSMIT_TIMEOUT_NS;
        self.retransmit_at_ns = now_ns.saturating_add(self.retransmit_timeout_ns);
        self.retransmissions = 0;
        transmissions.push(transmission);
        Ok(())
    }

    fn acknowledge_outstanding(&mut self, acknowledgment_number: u32) -> bool {
        let Some(outstanding) = self.outstanding.as_ref() else {
            return false;
        };
        if acknowledgment_number != outstanding.end_sequence_number {
            return false;
        }
        let flags = outstanding.flags;
        self.send_unacknowledged = acknowledgment_number;
        self.outstanding = None;
        self.retransmissions = 0;
        if flags & TCP_FLAG_FIN != 0 {
            self.state = match self.state {
                TcpState::FinWait1 => TcpState::FinWait2,
                TcpState::LastAck => TcpState::Closed,
                state => state,
            };
        }
        true
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
            window: (MAX_TCP_RECEIVE_WINDOW.saturating_sub(self.receive_buffer.len())) as u16,
            payload: copied_payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::{
        TcpSocketTable, TcpState, MAX_TCP_SEND_BUFFER_PER_SOCKET, MAX_TCP_SEND_BUFFER_TOTAL,
        MAX_TCP_SOCKETS,
    };
    use crate::kernel::net::tcp::segment::{
        write_segment, TcpSegment, TCP_FLAG_ACK, TCP_FLAG_SYN,
    };
    use crate::kernel::net::wire::Ipv4Address;

    #[test]
    fn client_enters_established_after_syn_ack() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets.connect(local, remote, 49152, 443, 1).unwrap();
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
        let handle = sockets.connect(local, remote, 49152, 443, 1).unwrap();
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
                .connect(
                    local,
                    remote,
                    49152 + index as u16,
                    443,
                    index as u64,
                )
                .unwrap();
        }
        assert_eq!(sockets.slots.len(), MAX_TCP_SOCKETS);
        assert!(matches!(
            sockets.connect(local, remote, 49408, 443, 1),
            Err(crate::kernel::error::Error::NoBufs)
        ));
    }

    #[test]
    fn send_buffer_limits_accept_partial_data_then_apply_backpressure() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets.connect(local, remote, 49152, 443, 1).unwrap();
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
                .connect(local, remote, 49152 + index as u16, 443, index as u64)
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
        let overflow_handle = sockets.connect(local, remote, 49216, 443, 1).unwrap();
        sockets.socket_mut(overflow_handle).unwrap().state = TcpState::Established;
        assert!(matches!(
            sockets.send(overflow_handle, b"x"),
            Err(crate::kernel::error::Error::NoBufs)
        ));
        assert!(!sockets.writable(overflow_handle));
    }

    #[test]
    fn listener_accepts_completed_handshake() {
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
}