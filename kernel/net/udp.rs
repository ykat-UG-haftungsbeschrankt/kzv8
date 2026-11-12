/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::kernel::error::Error;
use crate::kernel::net::wire::{ipv4_transport_checksum, Ipv4Address, IPV4_PROTOCOL_UDP};

pub(crate) const UDP_HEADER_LENGTH: usize = 8;
const MAX_UDP_SOCKETS: usize = 256;
const MAX_UDP_RECEIVE_DATAGRAMS_PER_SOCKET: usize = 64;
const MAX_UDP_RECEIVE_BYTES_PER_SOCKET: usize = 256 * 1024;
const MAX_UDP_RECEIVE_DATAGRAMS_TOTAL: usize = 1024;
const MAX_UDP_RECEIVE_BYTES_TOTAL: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UdpError {
    Truncated,
    InvalidLength,
    InvalidChecksum,
    LengthOverflow,
}

pub(crate) struct UdpDatagram<'a> {
    pub(crate) source_port: u16,
    pub(crate) destination_port: u16,
    pub(crate) payload: &'a [u8],
}

impl<'a> UdpDatagram<'a> {
    pub(crate) fn parse(
        bytes: &'a [u8],
        source: Ipv4Address,
        destination: Ipv4Address,
    ) -> Result<Self, UdpError> {
        if bytes.len() < UDP_HEADER_LENGTH {
            return Err(UdpError::Truncated);
        }
        let length = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
        if !(UDP_HEADER_LENGTH..=bytes.len()).contains(&length) {
            return Err(UdpError::InvalidLength);
        }
        let checksum = u16::from_be_bytes([bytes[6], bytes[7]]);
        if checksum != 0
            && ipv4_transport_checksum(source, destination, IPV4_PROTOCOL_UDP, &bytes[..length])
                != Some(0)
        {
            return Err(UdpError::InvalidChecksum);
        }
        Ok(Self {
            source_port: u16::from_be_bytes([bytes[0], bytes[1]]),
            destination_port: u16::from_be_bytes([bytes[2], bytes[3]]),
            payload: &bytes[UDP_HEADER_LENGTH..length],
        })
    }
}

pub(crate) fn write_datagram(
    output: &mut [u8],
    source: Ipv4Address,
    destination: Ipv4Address,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
) -> Result<usize, UdpError> {
    let length = UDP_HEADER_LENGTH
        .checked_add(payload.len())
        .ok_or(UdpError::LengthOverflow)?;
    let length_u16 = u16::try_from(length).map_err(|_| UdpError::LengthOverflow)?;
    if output.len() < length {
        return Err(UdpError::Truncated);
    }
    output[..UDP_HEADER_LENGTH].fill(0);
    output[0..2].copy_from_slice(&source_port.to_be_bytes());
    output[2..4].copy_from_slice(&destination_port.to_be_bytes());
    output[4..6].copy_from_slice(&length_u16.to_be_bytes());
    output[UDP_HEADER_LENGTH..length].copy_from_slice(payload);
    let checksum = ipv4_transport_checksum(source, destination, IPV4_PROTOCOL_UDP, &output[..length])
        .ok_or(UdpError::LengthOverflow)?;
    let checksum = if checksum == 0 { u16::MAX } else { checksum };
    output[6..8].copy_from_slice(&checksum.to_be_bytes());
    Ok(length)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UdpSocketHandle {
    index: usize,
    generation: u32,
}

pub(crate) struct UdpTransmission {
    pub(crate) source_address: Ipv4Address,
    pub(crate) destination_address: Ipv4Address,
    pub(crate) source_port: u16,
    pub(crate) destination_port: u16,
    pub(crate) payload: Vec<u8>,
}

struct ReceivedDatagram {
    source_address: Ipv4Address,
    source_port: u16,
    payload: Vec<u8>,
}

struct UdpSocket {
    local_address: Ipv4Address,
    local_port: u16,
    remote_endpoint: Option<(Ipv4Address, u16)>,
    received: VecDeque<ReceivedDatagram>,
    received_bytes: usize,
}

struct UdpSocketSlot {
    generation: u32,
    socket: Option<UdpSocket>,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct UdpFlowKey {
    local_port: u16,
    local_address: Ipv4Address,
    remote_endpoint: Option<(Ipv4Address, u16)>,
}

struct UdpFlowBucket {
    key: UdpFlowKey,
    slots: Vec<usize>,
}

pub(crate) struct UdpSocketTable {
    slots: Vec<UdpSocketSlot>,
    free_slots: Vec<usize>,
    flows: Vec<UdpFlowBucket>,
    queued_datagrams: usize,
    queued_bytes: usize,
}

impl UdpSocketTable {
    pub(crate) fn new() -> Option<Self> {
        let mut slots = Vec::new();
        slots.try_reserve_exact(MAX_UDP_SOCKETS).ok()?;
        let mut free_slots = Vec::new();
        free_slots.try_reserve_exact(MAX_UDP_SOCKETS).ok()?;
        let mut flows = Vec::new();
        flows.try_reserve_exact(MAX_UDP_SOCKETS).ok()?;
        Some(Self {
            slots,
            free_slots,
            flows,
            queued_datagrams: 0,
            queued_bytes: 0,
        })
    }

    pub(crate) fn create(&mut self) -> Result<UdpSocketHandle, Error> {
        let index = if let Some(index) = self.free_slots.pop() {
            index
        } else {
            if self.slots.len() >= MAX_UDP_SOCKETS {
                return Err(Error::NoBufs);
            }
            self.slots.push(UdpSocketSlot {
                generation: 0,
                socket: None,
            });
            self.slots.len() - 1
        };
        let generation = self.slots[index].generation.wrapping_add(1).max(1);
        self.slots[index].generation = generation;
        self.slots[index].socket = Some(UdpSocket {
            local_address: Ipv4Address([0; 4]),
            local_port: 0,
            remote_endpoint: None,
            received: VecDeque::new(),
            received_bytes: 0,
        });
        Ok(UdpSocketHandle { index, generation })
    }

    pub(crate) fn bind(
        &mut self,
        handle: UdpSocketHandle,
        address: Ipv4Address,
        port: u16,
    ) -> Result<(), Error> {
        if port == 0 {
            return Err(Error::Inval);
        }
        let index = self.slot_index(handle)?;
        if self.slots[index]
            .socket
            .as_ref()
            .ok_or(Error::BadFd)?
            .local_port
            != 0
        {
            return Err(Error::Inval);
        }
        let remote_endpoint = self.slots[index]
            .socket
            .as_ref()
            .ok_or(Error::BadFd)?
            .remote_endpoint;
        self.add_flow(
            UdpFlowKey {
                local_port: port,
                local_address: address,
                remote_endpoint,
            },
            index,
        )?;
        let socket = self.slots[index].socket.as_mut().ok_or(Error::BadFd)?;
        socket.local_address = address;
        socket.local_port = port;
        Ok(())
    }

    pub(crate) fn connect(
        &mut self,
        handle: UdpSocketHandle,
        address: Ipv4Address,
        port: u16,
    ) -> Result<(), Error> {
        if port == 0 {
            return Err(Error::Inval);
        }
        let index = self.slot_index(handle)?;
        let socket = self.slots[index].socket.as_ref().ok_or(Error::BadFd)?;
        let old_endpoint = socket.remote_endpoint;
        if socket.local_port != 0 {
            let old_key = UdpFlowKey {
                local_port: socket.local_port,
                local_address: socket.local_address,
                remote_endpoint: old_endpoint,
            };
            let new_key = UdpFlowKey {
                remote_endpoint: Some((address, port)),
                ..old_key
            };
            if old_key != new_key {
                self.add_flow(new_key, index)?;
                self.remove_flow(old_key, index);
            }
        }
        self.slots[index]
            .socket
            .as_mut()
            .ok_or(Error::BadFd)?
            .remote_endpoint = Some((address, port));
        Ok(())
    }

    pub(crate) fn send_to(
        &self,
        handle: UdpSocketHandle,
        local_address: Ipv4Address,
        destination_address: Ipv4Address,
        destination_port: u16,
        payload: &[u8],
    ) -> Result<UdpTransmission, Error> {
        if payload.len() > usize::from(u16::MAX) - UDP_HEADER_LENGTH {
            return Err(Error::MsgSize);
        }
        let socket = self.socket(handle)?;
        if socket.local_port == 0 || destination_port == 0 {
            return Err(Error::Inval);
        }
        if socket.remote_endpoint.is_some_and(|peer| {
            peer != (destination_address, destination_port)
        }) {
            return Err(Error::Inval);
        }
        let mut owned_payload = Vec::new();
        owned_payload
            .try_reserve_exact(payload.len())
            .map_err(|_| Error::NoMem)?;
        owned_payload.extend_from_slice(payload);
        Ok(UdpTransmission {
            source_address: if socket.local_address == Ipv4Address([0; 4]) {
                local_address
            } else {
                socket.local_address
            },
            destination_address,
            source_port: socket.local_port,
            destination_port,
            payload: owned_payload,
        })
    }

    pub(crate) fn receive_datagram(
        &mut self,
        source_address: Ipv4Address,
        destination_address: Ipv4Address,
        datagram: &UdpDatagram<'_>,
    ) -> Result<bool, Error> {
        let mut delivered = false;
        let mut queued_datagrams = self.queued_datagrams;
        let mut queued_bytes = self.queued_bytes;
        let payload_length = datagram.payload.len();
        let wildcard_address = Ipv4Address([0; 4]);
        let remote = Some((source_address, datagram.source_port));
        for local_address in [destination_address, wildcard_address] {
            if local_address == wildcard_address && destination_address == wildcard_address {
                continue;
            }
            for remote_endpoint in [remote, None] {
                let key = UdpFlowKey {
                    local_port: datagram.destination_port,
                    local_address,
                    remote_endpoint,
                };
                let Ok(bucket_index) = self.flows.binary_search_by_key(&key, |bucket| bucket.key) else {
                    continue;
                };
                for &index in &self.flows[bucket_index].slots {
                    let Some(socket) = self.slots[index].socket.as_mut() else {
                        continue;
                    };
                    if socket.received.len() >= MAX_UDP_RECEIVE_DATAGRAMS_PER_SOCKET
                        || socket.received_bytes.saturating_add(payload_length)
                            > MAX_UDP_RECEIVE_BYTES_PER_SOCKET
                        || queued_datagrams >= MAX_UDP_RECEIVE_DATAGRAMS_TOTAL
                        || queued_bytes
                            .checked_add(payload_length)
                            .is_none_or(|bytes| bytes > MAX_UDP_RECEIVE_BYTES_TOTAL)
                    {
                        continue;
                    }
                    let mut payload = Vec::new();
                    if payload.try_reserve_exact(payload_length).is_err() {
                        continue;
                    }
                    payload.extend_from_slice(datagram.payload);
                    if socket.received.try_reserve(1).is_err() {
                        continue;
                    }
                    socket.received.push_back(ReceivedDatagram {
                        source_address,
                        source_port: datagram.source_port,
                        payload,
                    });
                    socket.received_bytes += payload_length;
                    queued_datagrams += 1;
                    queued_bytes += payload_length;
                    delivered = true;
                }
            }
        }
        self.queued_datagrams = queued_datagrams;
        self.queued_bytes = queued_bytes;
        Ok(delivered)
    }

    pub(crate) fn receive(
        &mut self,
        handle: UdpSocketHandle,
        output: &mut [u8],
    ) -> Result<Option<(usize, Ipv4Address, u16)>, Error> {
        let (received, payload_length) = {
            let socket = self.socket_mut(handle)?;
            let Some(datagram) = socket.received.pop_front() else {
                return Ok(None);
            };
            let payload_length = datagram.payload.len();
            let length = output.len().min(payload_length);
            output[..length].copy_from_slice(&datagram.payload[..length]);
            socket.received_bytes = socket.received_bytes.saturating_sub(payload_length);
            (
                Some((length, datagram.source_address, datagram.source_port)),
                payload_length,
            )
        };
        self.queued_datagrams = self.queued_datagrams.saturating_sub(1);
        self.queued_bytes = self.queued_bytes.saturating_sub(payload_length);
        Ok(received)
    }

    pub(crate) fn close(&mut self, handle: UdpSocketHandle) -> bool {
        let Ok(index) = self.slot_index(handle) else {
            return false;
        };
        let Some(socket) = self.slots[index].socket.take() else {
            return false;
        };
        if socket.local_port != 0 {
            self.remove_flow(
                UdpFlowKey {
                    local_port: socket.local_port,
                    local_address: socket.local_address,
                    remote_endpoint: socket.remote_endpoint,
                },
                index,
            );
        }
        self.queued_datagrams = self.queued_datagrams.saturating_sub(socket.received.len());
        self.queued_bytes = self.queued_bytes.saturating_sub(socket.received_bytes);
        self.free_slots.push(index);
        true
    }

    pub(crate) fn endpoint(
        &self,
        handle: UdpSocketHandle,
    ) -> Result<(Ipv4Address, u16, Option<(Ipv4Address, u16)>), Error> {
        let socket = self.socket(handle)?;
        Ok((socket.local_address, socket.local_port, socket.remote_endpoint))
    }

    pub(crate) fn readable(&self, handle: UdpSocketHandle) -> bool {
        self.socket(handle)
            .map_or(false, |socket| !socket.received.is_empty())
    }

    fn slot_index(&self, handle: UdpSocketHandle) -> Result<usize, Error> {
        let slot = self.slots.get(handle.index).ok_or(Error::BadFd)?;
        if slot.generation == handle.generation && slot.socket.is_some() {
            Ok(handle.index)
        } else {
            Err(Error::BadFd)
        }
    }

    fn add_flow(&mut self, key: UdpFlowKey, index: usize) -> Result<(), Error> {
        match self.flows.binary_search_by_key(&key, |bucket| bucket.key) {
            Ok(bucket_index) => {
                self.flows[bucket_index]
                    .slots
                    .try_reserve(1)
                    .map_err(|_| Error::NoMem)?;
                self.flows[bucket_index].slots.push(index);
            }
            Err(bucket_index) => {
                let mut slots = Vec::new();
                slots.try_reserve_exact(1).map_err(|_| Error::NoMem)?;
                slots.push(index);
                self.flows.try_reserve(1).map_err(|_| Error::NoMem)?;
                self.flows.insert(bucket_index, UdpFlowBucket { key, slots });
            }
        }
        Ok(())
    }

    fn remove_flow(&mut self, key: UdpFlowKey, index: usize) {
        let Ok(bucket_index) = self.flows.binary_search_by_key(&key, |bucket| bucket.key) else {
            return;
        };
        let bucket = &mut self.flows[bucket_index];
        if let Some(slot_index) = bucket.slots.iter().position(|slot| *slot == index) {
            bucket.slots.remove(slot_index);
        }
        if bucket.slots.is_empty() {
            self.flows.remove(bucket_index);
        }
    }

    fn socket(&self, handle: UdpSocketHandle) -> Result<&UdpSocket, Error> {
        let index = self.slot_index(handle)?;
        self.slots[index].socket.as_ref().ok_or(Error::BadFd)
    }

    fn socket_mut(&mut self, handle: UdpSocketHandle) -> Result<&mut UdpSocket, Error> {
        let index = self.slot_index(handle)?;
        self.slots[index].socket.as_mut().ok_or(Error::BadFd)
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use super::{
        write_datagram, UdpDatagram, UdpSocketTable,
        MAX_UDP_RECEIVE_BYTES_PER_SOCKET, MAX_UDP_RECEIVE_DATAGRAMS_PER_SOCKET,
        MAX_UDP_SOCKETS,
    };
    use crate::kernel::net::wire::Ipv4Address;

    #[test]
    fn datagram_round_trip_preserves_ports_payload_and_checksum() {
        let client = Ipv4Address([192, 0, 2, 1]);
        let server = Ipv4Address([192, 0, 2, 2]);
        let payload = b"hello over udp";
        let mut bytes = [0; 64];
        let length = write_datagram(&mut bytes, client, server, 49152, 53, payload).unwrap();
        let datagram = UdpDatagram::parse(&bytes[..length], client, server).unwrap();
        assert_eq!(datagram.source_port, 49152);
        assert_eq!(datagram.destination_port, 53);
        assert_eq!(datagram.payload, payload);
    }

    #[test]
    fn bound_socket_receives_datagram_with_source_endpoint() {
        let client = Ipv4Address([192, 0, 2, 1]);
        let server = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = UdpSocketTable::new().unwrap();
        let socket = sockets.create().unwrap();
        sockets.bind(socket, server, 53).unwrap();
        let mut bytes = [0; 64];
        let length = write_datagram(&mut bytes, client, server, 49152, 53, b"query").unwrap();
        let datagram = UdpDatagram::parse(&bytes[..length], client, server).unwrap();
        assert!(sockets
            .receive_datagram(client, server, &datagram)
            .unwrap());

        let mut output = Vec::new();
        output.resize(16, 0);
        let received = sockets.receive(socket, &mut output).unwrap().unwrap();
        assert_eq!(received, (5, client, 49152));
        assert_eq!(&output[..5], b"query");
    }

    #[test]
    fn flow_index_matches_connected_and_wildcard_sockets_only() {
        let client = Ipv4Address([192, 0, 2, 1]);
        let other_client = Ipv4Address([192, 0, 2, 3]);
        let server = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = UdpSocketTable::new().unwrap();
        let connected = sockets.create().unwrap();
        let wildcard = sockets.create().unwrap();
        let other = sockets.create().unwrap();
        sockets.bind(connected, server, 53).unwrap();
        sockets.connect(connected, client, 49152).unwrap();
        sockets.bind(wildcard, Ipv4Address([0; 4]), 53).unwrap();
        sockets.bind(other, server, 53).unwrap();
        sockets.connect(other, other_client, 49152).unwrap();

        let datagram = UdpDatagram {
            source_port: 49152,
            destination_port: 53,
            payload: b"query",
        };
        assert!(sockets
            .receive_datagram(client, server, &datagram)
            .unwrap());
        assert!(sockets.readable(connected));
        assert!(sockets.readable(wildcard));
        assert!(!sockets.readable(other));
    }

    #[test]
    fn socket_table_grows_beyond_initial_capacity() {
        let mut sockets = UdpSocketTable::new().unwrap();
        for _ in 0..MAX_UDP_SOCKETS {
            sockets.create().unwrap();
        }
        assert_eq!(sockets.slots.len(), MAX_UDP_SOCKETS);
        assert!(matches!(
            sockets.create(),
            Err(crate::kernel::error::Error::NoBufs)
        ));
    }

    #[test]
    fn receive_queue_drops_at_quota_and_recovers_after_dequeue() {
        let client = Ipv4Address([192, 0, 2, 1]);
        let server = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = UdpSocketTable::new().unwrap();
        let socket = sockets.create().unwrap();
        sockets.bind(socket, server, 53).unwrap();
        let mut bytes = [0; 32];
        let length = write_datagram(&mut bytes, client, server, 49152, 53, b"x").unwrap();
        let datagram = UdpDatagram::parse(&bytes[..length], client, server).unwrap();

        for _ in 0..MAX_UDP_RECEIVE_DATAGRAMS_PER_SOCKET {
            assert!(sockets
                .receive_datagram(client, server, &datagram)
                .unwrap());
        }
        assert!(!sockets
            .receive_datagram(client, server, &datagram)
            .unwrap());
        assert_eq!(sockets.queued_datagrams, MAX_UDP_RECEIVE_DATAGRAMS_PER_SOCKET);

        let mut output = [0; 1];
        assert!(sockets.receive(socket, &mut output).unwrap().is_some());
        assert!(sockets
            .receive_datagram(client, server, &datagram)
            .unwrap());
        assert_eq!(sockets.queued_datagrams, MAX_UDP_RECEIVE_DATAGRAMS_PER_SOCKET);
        assert!(sockets.close(socket));
        assert_eq!(sockets.queued_datagrams, 0);
        assert_eq!(sockets.queued_bytes, 0);
    }

    #[test]
    fn receive_queue_drops_at_byte_quota() {
        let client = Ipv4Address([192, 0, 2, 1]);
        let server = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = UdpSocketTable::new().unwrap();
        let socket = sockets.create().unwrap();
        sockets.bind(socket, server, 53).unwrap();
        let payload_length = MAX_UDP_RECEIVE_BYTES_PER_SOCKET / 64 + 1;
        let mut payload = Vec::new();
        payload.resize(payload_length, 0x7a);
        let datagram = UdpDatagram {
            source_port: 49152,
            destination_port: 53,
            payload: &payload,
        };

        let accepted_datagrams = MAX_UDP_RECEIVE_BYTES_PER_SOCKET / payload_length;
        for _ in 0..accepted_datagrams {
            assert!(sockets
                .receive_datagram(client, server, &datagram)
                .unwrap());
        }
        assert!(!sockets
            .receive_datagram(client, server, &datagram)
            .unwrap());
        assert_eq!(
            sockets.queued_bytes,
            accepted_datagrams * payload_length
        );
        assert!(sockets.queued_bytes <= MAX_UDP_RECEIVE_BYTES_PER_SOCKET);
    }
}