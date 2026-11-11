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

pub(crate) struct UdpSocketTable {
    slots: Vec<UdpSocketSlot>,
    queued_datagrams: usize,
    queued_bytes: usize,
}

impl UdpSocketTable {
    pub(crate) fn new() -> Option<Self> {
        Some(Self {
            slots: Vec::new(),
            queued_datagrams: 0,
            queued_bytes: 0,
        })
    }

    pub(crate) fn create(&mut self) -> Result<UdpSocketHandle, Error> {
        let index = if let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.socket.is_none())
        {
            index
        } else {
            if self.slots.len() >= MAX_UDP_SOCKETS {
                return Err(Error::NoBufs);
            }
            self.slots.try_reserve(1).map_err(|_| Error::NoMem)?;
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
        let socket = self.slots[index].socket.as_mut().ok_or(Error::BadFd)?;
        if socket.local_port != 0 {
            return Err(Error::Inval);
        }
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
        self.socket_mut(handle)?.remote_endpoint = Some((address, port));
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
        for socket in self.slots.iter_mut().filter_map(|slot| slot.socket.as_mut()) {
            if socket.local_port != datagram.destination_port
                || !(socket.local_address == destination_address
                    || socket.local_address == Ipv4Address([0; 4]))
                || !socket.remote_endpoint.map_or(true, |peer| {
                    peer == (source_address, datagram.source_port)
                })
            {
                continue;
            }
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
        self.queued_datagrams = self.queued_datagrams.saturating_sub(socket.received.len());
        self.queued_bytes = self.queued_bytes.saturating_sub(socket.received_bytes);
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