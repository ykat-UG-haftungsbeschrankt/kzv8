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
}

struct UdpSocketSlot {
    generation: u32,
    socket: Option<UdpSocket>,
}

pub(crate) struct UdpSocketTable {
    slots: Vec<UdpSocketSlot>,
}

impl UdpSocketTable {
    pub(crate) fn new() -> Option<Self> {
        Some(Self { slots: Vec::new() })
    }

    pub(crate) fn create(&mut self) -> Result<UdpSocketHandle, Error> {
        let index = if let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.socket.is_none())
        {
            index
        } else {
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
            let mut payload = Vec::new();
            payload
                .try_reserve_exact(datagram.payload.len())
                .map_err(|_| Error::NoMem)?;
            payload.extend_from_slice(datagram.payload);
            socket.received.try_reserve(1).map_err(|_| Error::NoMem)?;
            socket.received.push_back(ReceivedDatagram {
                source_address,
                source_port: datagram.source_port,
                payload,
            });
            delivered = true;
        }
        Ok(delivered)
    }

    pub(crate) fn receive(
        &mut self,
        handle: UdpSocketHandle,
        output: &mut [u8],
    ) -> Result<Option<(usize, Ipv4Address, u16)>, Error> {
        let socket = self.socket_mut(handle)?;
        let Some(datagram) = socket.received.pop_front() else {
            return Ok(None);
        };
        let length = output.len().min(datagram.payload.len());
        output[..length].copy_from_slice(&datagram.payload[..length]);
        Ok(Some((length, datagram.source_address, datagram.source_port)))
    }

    pub(crate) fn close(&mut self, handle: UdpSocketHandle) -> bool {
        let Ok(index) = self.slot_index(handle) else {
            return false;
        };
        self.slots[index].socket = None;
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

    use super::{write_datagram, UdpDatagram, UdpSocketTable};
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
        for _ in 0..32 {
            sockets.create().unwrap();
        }
        assert_eq!(sockets.slots.len(), 32);
    }
}