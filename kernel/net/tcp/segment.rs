/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::net::wire::{ipv4_transport_checksum, Ipv4Address, IPV4_PROTOCOL_TCP};

pub(crate) const TCP_HEADER_LENGTH: usize = 20;
pub(crate) const TCP_FLAG_FIN: u8 = 0x01;
pub(crate) const TCP_FLAG_SYN: u8 = 0x02;
pub(crate) const TCP_FLAG_RST: u8 = 0x04;
pub(crate) const TCP_FLAG_PSH: u8 = 0x08;
pub(crate) const TCP_FLAG_ACK: u8 = 0x10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TcpSegmentError {
    Truncated,
    InvalidHeaderLength,
    InvalidChecksum,
    LengthOverflow,
}

pub(crate) struct TcpSegment<'a> {
    pub(crate) source_port: u16,
    pub(crate) destination_port: u16,
    pub(crate) sequence_number: u32,
    pub(crate) acknowledgment_number: u32,
    pub(crate) flags: u8,
    pub(crate) window: u16,
    pub(crate) payload: &'a [u8],
}

impl<'a> TcpSegment<'a> {
    pub(crate) fn parse(
        bytes: &'a [u8],
        source: Ipv4Address,
        destination: Ipv4Address,
    ) -> Result<Self, TcpSegmentError> {
        if bytes.len() < TCP_HEADER_LENGTH {
            return Err(TcpSegmentError::Truncated);
        }
        let header_length = usize::from(bytes[12] >> 4) * 4;
        if !(TCP_HEADER_LENGTH..=bytes.len()).contains(&header_length) {
            return Err(TcpSegmentError::InvalidHeaderLength);
        }
        if ipv4_transport_checksum(source, destination, IPV4_PROTOCOL_TCP, bytes) != Some(0) {
            return Err(TcpSegmentError::InvalidChecksum);
        }
        Ok(Self {
            source_port: u16::from_be_bytes([bytes[0], bytes[1]]),
            destination_port: u16::from_be_bytes([bytes[2], bytes[3]]),
            sequence_number: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            acknowledgment_number: u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            flags: bytes[13],
            window: u16::from_be_bytes([bytes[14], bytes[15]]),
            payload: &bytes[header_length..],
        })
    }
}

pub(crate) fn write_segment(
    output: &mut [u8],
    source: Ipv4Address,
    destination: Ipv4Address,
    source_port: u16,
    destination_port: u16,
    sequence_number: u32,
    acknowledgment_number: u32,
    flags: u8,
    window: u16,
    payload: &[u8],
) -> Result<usize, TcpSegmentError> {
    let length = TCP_HEADER_LENGTH
        .checked_add(payload.len())
        .ok_or(TcpSegmentError::LengthOverflow)?;
    u16::try_from(length).map_err(|_| TcpSegmentError::LengthOverflow)?;
    if output.len() < length {
        return Err(TcpSegmentError::Truncated);
    }
    output[..TCP_HEADER_LENGTH].fill(0);
    output[0..2].copy_from_slice(&source_port.to_be_bytes());
    output[2..4].copy_from_slice(&destination_port.to_be_bytes());
    output[4..8].copy_from_slice(&sequence_number.to_be_bytes());
    output[8..12].copy_from_slice(&acknowledgment_number.to_be_bytes());
    output[12] = 5 << 4;
    output[13] = flags;
    output[14..16].copy_from_slice(&window.to_be_bytes());
    output[TCP_HEADER_LENGTH..length].copy_from_slice(payload);
    let checksum = ipv4_transport_checksum(source, destination, IPV4_PROTOCOL_TCP, &output[..length])
        .ok_or(TcpSegmentError::LengthOverflow)?;
    output[16..18].copy_from_slice(&checksum.to_be_bytes());
    Ok(length)
}

#[cfg(test)]
#[path = "../../../tests/kernel/net/tcp/segment.rs"]
mod tests;