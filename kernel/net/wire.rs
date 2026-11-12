/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use crate::kernel::error::Error;

pub(crate) const ETHERNET_HEADER_LENGTH: usize = 14;
pub(crate) const IPV4_MINIMUM_HEADER_LENGTH: usize = 20;
pub(crate) const IPV4_MAXIMUM_HEADER_LENGTH: usize = 60;
const MAX_IPV4_FRAGMENT_COUNT: usize = (u16::MAX as usize - IPV4_MINIMUM_HEADER_LENGTH + 7) / 8;
pub(crate) const ETHER_TYPE_IPV4: u16 = 0x0800;
pub(crate) const ETHER_TYPE_ARP: u16 = 0x0806;
pub(crate) const IPV4_PROTOCOL_ICMP: u8 = 1;
pub(crate) const IPV4_PROTOCOL_TCP: u8 = 6;
pub(crate) const IPV4_PROTOCOL_UDP: u8 = 17;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MacAddress(pub(crate) [u8; 6]);

impl MacAddress {
    pub(crate) const BROADCAST: Self = Self([0xff; 6]);

    pub(crate) const fn is_multicast(self) -> bool {
        self.0[0] & 1 != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Ipv4Address(pub(crate) [u8; 4]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WireError {
    Truncated,
    InvalidEthernetAddress,
    InvalidIpVersion,
    InvalidIpHeaderLength,
    InvalidIpTotalLength,
    InvalidIpChecksum,
    InvalidIpTtl,
    LengthOverflow,
}

pub(crate) struct EthernetFrame<'a> {
    pub(crate) destination: MacAddress,
    pub(crate) source: MacAddress,
    pub(crate) ether_type: u16,
    pub(crate) payload: &'a [u8],
}

impl<'a> EthernetFrame<'a> {
    pub(crate) fn parse(bytes: &'a [u8]) -> Result<Self, WireError> {
        if bytes.len() < ETHERNET_HEADER_LENGTH {
            return Err(WireError::Truncated);
        }
        let destination = bytes[..6]
            .try_into()
            .map(MacAddress)
            .map_err(|_| WireError::InvalidEthernetAddress)?;
        let source = bytes[6..12]
            .try_into()
            .map(MacAddress)
            .map_err(|_| WireError::InvalidEthernetAddress)?;
        let ether_type = u16::from_be_bytes([bytes[12], bytes[13]]);
        Ok(Self {
            destination,
            source,
            ether_type,
            payload: &bytes[ETHERNET_HEADER_LENGTH..],
        })
    }
}

pub(crate) struct Ipv4Packet<'a> {
    pub(crate) header: &'a [u8],
    pub(crate) source: Ipv4Address,
    pub(crate) destination: Ipv4Address,
    pub(crate) protocol: u8,
    pub(crate) ttl: u8,
    pub(crate) identification: u16,
    pub(crate) dont_fragment: bool,
    pub(crate) more_fragments: bool,
    pub(crate) fragment_offset: u16,
    pub(crate) payload: &'a [u8],
}

impl<'a> Ipv4Packet<'a> {
    pub(crate) fn parse(bytes: &'a [u8]) -> Result<Self, WireError> {
        if bytes.len() < IPV4_MINIMUM_HEADER_LENGTH {
            return Err(WireError::Truncated);
        }
        if bytes[0] >> 4 != 4 {
            return Err(WireError::InvalidIpVersion);
        }
        let header_length = usize::from(bytes[0] & 0x0f) * 4;
        if !(IPV4_MINIMUM_HEADER_LENGTH..=IPV4_MAXIMUM_HEADER_LENGTH)
            .contains(&header_length)
        {
            return Err(WireError::InvalidIpHeaderLength);
        }
        if bytes.len() < header_length {
            return Err(WireError::Truncated);
        }
        let total_length = usize::from(u16::from_be_bytes([bytes[2], bytes[3]]));
        if total_length < header_length || total_length > bytes.len() {
            return Err(WireError::InvalidIpTotalLength);
        }
        if internet_checksum(&bytes[..header_length]) != 0 {
            return Err(WireError::InvalidIpChecksum);
        }

        let fragment = u16::from_be_bytes([bytes[6], bytes[7]]);
        let mut source = [0; 4];
        source.copy_from_slice(&bytes[12..16]);
        let mut destination = [0; 4];
        destination.copy_from_slice(&bytes[16..20]);
        Ok(Self {
            header: &bytes[..header_length],
            source: Ipv4Address(source),
            destination: Ipv4Address(destination),
            protocol: bytes[9],
            ttl: bytes[8],
            identification: u16::from_be_bytes([bytes[4], bytes[5]]),
            dont_fragment: fragment & 0x4000 != 0,
            more_fragments: fragment & 0x2000 != 0,
            fragment_offset: fragment & 0x1fff,
            payload: &bytes[header_length..total_length],
        })
    }
}

pub(crate) fn rewrite_forwarded_ipv4_header(
    output: &mut [u8],
    input: &[u8],
) -> Result<usize, WireError> {
    if input.len() < IPV4_MINIMUM_HEADER_LENGTH || input[0] >> 4 != 4 {
        return Err(WireError::InvalidIpVersion);
    }
    let header_length = usize::from(input[0] & 0x0f) * 4;
    if !(IPV4_MINIMUM_HEADER_LENGTH..=IPV4_MAXIMUM_HEADER_LENGTH).contains(&header_length) {
        return Err(WireError::InvalidIpHeaderLength);
    }
    if input.len() < header_length || output.len() < header_length {
        return Err(WireError::Truncated);
    }
    if input[8] <= 1 {
        return Err(WireError::InvalidIpTtl);
    }
    output[..header_length].copy_from_slice(&input[..header_length]);
    output[8] -= 1;
    output[10..12].fill(0);
    let checksum = internet_checksum(&output[..header_length]);
    output[10..12].copy_from_slice(&checksum.to_be_bytes());
    Ok(header_length)
}

pub(crate) fn fragment_ipv4_packet(packet: &Ipv4Packet<'_>, mtu: usize) -> Result<Vec<Vec<u8>>, Error> {
    if packet.dont_fragment
        || packet.payload.is_empty()
        || packet.more_fragments && packet.payload.len() % 8 != 0
        || mtu <= IPV4_MINIMUM_HEADER_LENGTH
    {
        return Err(Error::MsgSize);
    }
    let copied_options = copied_ipv4_options(packet.header)?;
    let base_offset = usize::from(packet.fragment_offset)
        .checked_mul(8)
        .ok_or(Error::MsgSize)?;
    let total_payload_end = base_offset
        .checked_add(packet.payload.len())
        .ok_or(Error::MsgSize)?;
    if total_payload_end > usize::from(u16::MAX) - IPV4_MINIMUM_HEADER_LENGTH {
        return Err(Error::MsgSize);
    }

    let mut fragments = Vec::new();
    let mut payload_offset = 0usize;
    while payload_offset < packet.payload.len() {
        if fragments.len() >= MAX_IPV4_FRAGMENT_COUNT {
            return Err(Error::MsgSize);
        }
        let fragment_offset = base_offset
            .checked_add(payload_offset)
            .ok_or(Error::MsgSize)?;
        let offset_units = fragment_offset / 8;
        if fragment_offset % 8 != 0 || offset_units > 0x1fff {
            return Err(Error::MsgSize);
        }
        let use_full_header = fragment_offset == 0;
        let options = if use_full_header {
            &packet.header[IPV4_MINIMUM_HEADER_LENGTH..]
        } else {
            copied_options.as_slice()
        };
        let header_length = IPV4_MINIMUM_HEADER_LENGTH
            .checked_add(options.len())
            .ok_or(Error::MsgSize)?;
        let maximum_payload = mtu.checked_sub(header_length).ok_or(Error::MsgSize)?;
        if maximum_payload == 0 {
            return Err(Error::MsgSize);
        }
        let remaining = packet.payload.len() - payload_offset;
        let mut fragment_payload_length = remaining.min(maximum_payload);
        if fragment_payload_length < remaining {
            fragment_payload_length &= !7;
            if fragment_payload_length == 0 {
                return Err(Error::MsgSize);
            }
        }
        let more_fragments = fragment_payload_length < remaining || packet.more_fragments;
        let fragment_length = header_length
            .checked_add(fragment_payload_length)
            .ok_or(Error::MsgSize)?;
        let fragment_length_u16 = u16::try_from(fragment_length).map_err(|_| Error::MsgSize)?;
        let mut fragment = Vec::new();
        fragment
            .try_reserve_exact(fragment_length)
            .map_err(|_| Error::NoMem)?;
        fragment.resize(fragment_length, 0);
        fragment[..IPV4_MINIMUM_HEADER_LENGTH]
            .copy_from_slice(&packet.header[..IPV4_MINIMUM_HEADER_LENGTH]);
        fragment[0] = (packet.header[0] & 0xf0) | (header_length as u8 / 4);
        fragment[IPV4_MINIMUM_HEADER_LENGTH..header_length].copy_from_slice(options);
        fragment[header_length..]
            .copy_from_slice(&packet.payload[payload_offset..payload_offset + fragment_payload_length]);
        fragment[2..4].copy_from_slice(&fragment_length_u16.to_be_bytes());
        let reserved = u16::from_be_bytes([packet.header[6], packet.header[7]]) & 0x8000;
        let fragment_field = reserved
            | if more_fragments { 0x2000 } else { 0 }
            | u16::try_from(offset_units).map_err(|_| Error::MsgSize)?;
        fragment[6..8].copy_from_slice(&fragment_field.to_be_bytes());
        fragment[10..12].fill(0);
        let checksum = internet_checksum(&fragment[..header_length]);
        fragment[10..12].copy_from_slice(&checksum.to_be_bytes());
        fragments.try_reserve(1).map_err(|_| Error::NoMem)?;
        fragments.push(fragment);
        payload_offset = payload_offset
            .checked_add(fragment_payload_length)
            .ok_or(Error::MsgSize)?;
    }
    Ok(fragments)
}

fn copied_ipv4_options(header: &[u8]) -> Result<Vec<u8>, Error> {
    if header.len() < IPV4_MINIMUM_HEADER_LENGTH {
        return Err(Error::Proto);
    }
    let options = &header[IPV4_MINIMUM_HEADER_LENGTH..];
    let mut copied = Vec::new();
    copied.try_reserve_exact(options.len()).map_err(|_| Error::NoMem)?;
    let mut offset = 0usize;
    while offset < options.len() {
        let kind = options[offset];
        if kind == 0 {
            break;
        }
        if kind == 1 {
            offset += 1;
            continue;
        }
        let length = usize::from(*options.get(offset + 1).ok_or(Error::Proto)?);
        if length < 2 || offset.checked_add(length).is_none_or(|end| end > options.len()) {
            return Err(Error::Proto);
        }
        if kind & 0x80 != 0 {
            copied.extend_from_slice(&options[offset..offset + length]);
        }
        offset += length;
    }
    while copied.len() % 4 != 0 {
        copied.push(0);
    }
    Ok(copied)
}

pub(crate) fn write_ethernet_header(
    output: &mut [u8],
    destination: MacAddress,
    source: MacAddress,
    ether_type: u16,
) -> Result<(), WireError> {
    if output.len() < ETHERNET_HEADER_LENGTH {
        return Err(WireError::Truncated);
    }
    output[..6].copy_from_slice(&destination.0);
    output[6..12].copy_from_slice(&source.0);
    output[12..14].copy_from_slice(&ether_type.to_be_bytes());
    Ok(())
}

pub(crate) fn write_ipv4_header(
    output: &mut [u8],
    source: Ipv4Address,
    destination: Ipv4Address,
    protocol: u8,
    ttl: u8,
    identification: u16,
    flags_and_fragment_offset: u16,
    payload_length: usize,
) -> Result<usize, WireError> {
    let total_length = IPV4_MINIMUM_HEADER_LENGTH
        .checked_add(payload_length)
        .ok_or(WireError::LengthOverflow)?;
    if total_length > usize::from(u16::MAX) {
        return Err(WireError::LengthOverflow);
    }
    if output.len() < total_length {
        return Err(WireError::Truncated);
    }

    output[..IPV4_MINIMUM_HEADER_LENGTH].fill(0);
    output[0] = 0x45;
    output[2..4].copy_from_slice(&(total_length as u16).to_be_bytes());
    output[4..6].copy_from_slice(&identification.to_be_bytes());
    output[6..8].copy_from_slice(&flags_and_fragment_offset.to_be_bytes());
    output[8] = ttl;
    output[9] = protocol;
    output[12..16].copy_from_slice(&source.0);
    output[16..20].copy_from_slice(&destination.0);
    let checksum = internet_checksum(&output[..IPV4_MINIMUM_HEADER_LENGTH]);
    output[10..12].copy_from_slice(&checksum.to_be_bytes());
    Ok(total_length)
}

pub(crate) fn internet_checksum(bytes: &[u8]) -> u16 {
    fold_checksum(checksum_sum(bytes, 0))
}

pub(crate) fn ipv4_transport_checksum(
    source: Ipv4Address,
    destination: Ipv4Address,
    protocol: u8,
    segment: &[u8],
) -> Option<u16> {
    let segment_length = u16::try_from(segment.len()).ok()?;
    let mut sum = checksum_sum(&source.0, 0);
    sum = checksum_sum(&destination.0, sum);
    sum += u64::from(protocol);
    sum += u64::from(segment_length);
    Some(fold_checksum(checksum_sum(segment, sum)))
}

fn checksum_sum(bytes: &[u8], mut sum: u64) -> u64 {
    let mut words = bytes.chunks_exact(2);
    for word in &mut words {
        sum += u64::from(u16::from_be_bytes([word[0], word[1]]));
    }
    if let Some(byte) = words.remainder().first() {
        sum += u64::from(u16::from_be_bytes([*byte, 0]));
    }
    sum
}

fn fold_checksum(mut sum: u64) -> u16 {
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::{
        fragment_ipv4_packet, rewrite_forwarded_ipv4_header, write_ipv4_header, Ipv4Address,
        Ipv4Packet, IPV4_PROTOCOL_UDP,
    };
    use crate::kernel::error::Error;
    use crate::kernel::net::ipv4_fragments::Ipv4FragmentReassembler;

    #[test]
    fn forwarding_decrements_ttl_and_repairs_checksum() {
        let source = Ipv4Address([192, 0, 2, 1]);
        let destination = Ipv4Address([198, 51, 100, 8]);
        let mut original = [0; 20];
        write_ipv4_header(
            &mut original,
            source,
            destination,
            IPV4_PROTOCOL_UDP,
            64,
            17,
            0,
            0,
        )
        .unwrap();
        let mut forwarded = [0; 20];
        rewrite_forwarded_ipv4_header(&mut forwarded, &original).unwrap();
        let packet = Ipv4Packet::parse(&forwarded).unwrap();
        assert_eq!(packet.ttl, 63);
        assert_eq!(packet.source, source);
        assert_eq!(packet.destination, destination);
    }

    #[test]
    fn ipv4_fragments_reassemble_out_of_order() {
        let source = Ipv4Address([192, 0, 2, 1]);
        let destination = Ipv4Address([198, 51, 100, 8]);
        let mut original = [0; 60];
        write_ipv4_header(
            &mut original,
            source,
            destination,
            IPV4_PROTOCOL_UDP,
            64,
            41,
            0,
            40,
        )
        .unwrap();
        for (index, byte) in original[20..].iter_mut().enumerate() {
            *byte = index as u8;
        }
        let packet = Ipv4Packet::parse(&original).unwrap();
        let fragments = fragment_ipv4_packet(&packet, 36).unwrap();
        assert_eq!(fragments.len(), 3);

        let mut reassembler = Ipv4FragmentReassembler::new();
        let mut complete = None;
        for (index, bytes) in fragments.iter().rev().enumerate() {
            let fragment = Ipv4Packet::parse(bytes).unwrap();
            complete = reassembler
                .insert(&fragment, index as u64)
                .unwrap()
                .or(complete);
        }
        let reassembled = complete.unwrap();
        let parsed = Ipv4Packet::parse(&reassembled).unwrap();
        assert_eq!(parsed.source, source);
        assert_eq!(parsed.destination, destination);
        assert_eq!(parsed.identification, 41);
        assert_eq!(parsed.fragment_offset, 0);
        assert!(!parsed.more_fragments);
        assert_eq!(parsed.payload, &original[20..]);
    }

    #[test]
    fn ipv4_fragmenter_honors_dont_fragment() {
        let source = Ipv4Address([192, 0, 2, 1]);
        let destination = Ipv4Address([198, 51, 100, 8]);
        let mut original = [0; 60];
        write_ipv4_header(
            &mut original,
            source,
            destination,
            IPV4_PROTOCOL_UDP,
            64,
            42,
            0x4000,
            40,
        )
        .unwrap();
        let packet = Ipv4Packet::parse(&original).unwrap();
        assert_eq!(
            fragment_ipv4_packet(&packet, 36),
            Err(Error::MsgSize)
        );
    }

    #[test]
    fn fragment_reassembly_expires_from_first_arrival() {
        let source = Ipv4Address([192, 0, 2, 1]);
        let destination = Ipv4Address([198, 51, 100, 8]);
        let mut original = [0; 60];
        write_ipv4_header(
            &mut original,
            source,
            destination,
            IPV4_PROTOCOL_UDP,
            64,
            43,
            0,
            40,
        )
        .unwrap();
        let packet = Ipv4Packet::parse(&original).unwrap();
        let fragments = fragment_ipv4_packet(&packet, 36).unwrap();
        let first = Ipv4Packet::parse(&fragments[0]).unwrap();
        let middle = Ipv4Packet::parse(&fragments[1]).unwrap();
        let last = Ipv4Packet::parse(fragments.last().unwrap()).unwrap();
        let mut reassembler = Ipv4FragmentReassembler::new();

        assert!(reassembler.insert(&first, 0).unwrap().is_none());
        assert!(reassembler
            .insert(&middle, 20_000_000_000)
            .unwrap()
            .is_none());
        assert!(reassembler
            .insert(&last, 31_000_000_000)
            .unwrap()
            .is_none());
    }
}