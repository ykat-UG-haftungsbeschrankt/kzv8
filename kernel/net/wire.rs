/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) const ETHERNET_HEADER_LENGTH: usize = 14;
pub(crate) const IPV4_MINIMUM_HEADER_LENGTH: usize = 20;
pub(crate) const IPV4_MAXIMUM_HEADER_LENGTH: usize = 60;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
        rewrite_forwarded_ipv4_header, write_ipv4_header, Ipv4Address, Ipv4Packet,
        IPV4_PROTOCOL_UDP,
    };

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
}