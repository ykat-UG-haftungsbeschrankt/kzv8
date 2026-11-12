/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::error::Error;
use crate::kernel::net::wire::internet_checksum;

pub(crate) const ICMP_ECHO_REPLY: u8 = 0;
pub(crate) const ICMP_ECHO_REQUEST: u8 = 8;
pub(crate) const ICMP_DESTINATION_UNREACHABLE: u8 = 3;
pub(crate) const ICMP_TIME_EXCEEDED: u8 = 11;
const ICMP_ECHO_HEADER_LENGTH: usize = 8;

pub(crate) fn is_error_message(message: &[u8]) -> bool {
    matches!(message.first().copied(), Some(3 | 4 | 5 | 11 | 12))
}

pub(crate) fn write_echo_reply(request: &[u8], output: &mut [u8]) -> Result<usize, Error> {
    if request.len() < ICMP_ECHO_HEADER_LENGTH || output.len() < request.len() {
        return Err(Error::MsgSize);
    }
    if request[0] != ICMP_ECHO_REQUEST || request[1] != 0 {
        return Err(Error::Proto);
    }
    if internet_checksum(request) != 0 {
        return Err(Error::Proto);
    }

    output[..request.len()].copy_from_slice(request);
    output[0] = ICMP_ECHO_REPLY;
    output[2..4].fill(0);
    let checksum = internet_checksum(&output[..request.len()]);
    output[2..4].copy_from_slice(&checksum.to_be_bytes());
    Ok(request.len())
}

pub(crate) fn write_time_exceeded(
    original_header: &[u8],
    original_payload: &[u8],
    output: &mut [u8],
) -> Result<usize, Error> {
    write_error(
        ICMP_TIME_EXCEEDED,
        0,
        original_header,
        original_payload,
        output,
    )
}

pub(crate) fn write_destination_unreachable(
    code: u8,
    original_header: &[u8],
    original_payload: &[u8],
    output: &mut [u8],
) -> Result<usize, Error> {
    write_error(
        ICMP_DESTINATION_UNREACHABLE,
        code,
        original_header,
        original_payload,
        output,
    )
}

pub(crate) fn write_fragmentation_needed(
    original_header: &[u8],
    original_payload: &[u8],
    next_hop_mtu: u16,
    output: &mut [u8],
) -> Result<usize, Error> {
    let length = write_error(
        ICMP_DESTINATION_UNREACHABLE,
        4,
        original_header,
        original_payload,
        output,
    )?;
    output[6..8].copy_from_slice(&next_hop_mtu.to_be_bytes());
    output[2..4].fill(0);
    let checksum = internet_checksum(&output[..length]);
    output[2..4].copy_from_slice(&checksum.to_be_bytes());
    Ok(length)
}

fn write_error(
    message_type: u8,
    code: u8,
    original_header: &[u8],
    original_payload: &[u8],
    output: &mut [u8],
) -> Result<usize, Error> {
    if original_header.len() < 20 {
        return Err(Error::MsgSize);
    }
    let quote_length = original_payload.len().min(8);
    let length = ICMP_ECHO_HEADER_LENGTH + original_header.len() + quote_length;
    if output.len() < length {
        return Err(Error::MsgSize);
    }
    output[..length].fill(0);
    output[0] = message_type;
    output[1] = code;
    let header_end = ICMP_ECHO_HEADER_LENGTH + original_header.len();
    output[ICMP_ECHO_HEADER_LENGTH..header_end].copy_from_slice(original_header);
    output[header_end..length].copy_from_slice(&original_payload[..quote_length]);
    let checksum = internet_checksum(&output[..length]);
    output[2..4].copy_from_slice(&checksum.to_be_bytes());
    Ok(length)
}

#[cfg(test)]
mod tests {
    use super::write_fragmentation_needed;
    use crate::kernel::net::wire::internet_checksum;

    #[test]
    fn fragmentation_needed_checksum_covers_next_hop_mtu() {
        let mut original_header = [0; 20];
        original_header[0] = 0x45;
        let mut output = [0; 36];
        let length = write_fragmentation_needed(
            &original_header,
            &[0; 8],
            1400,
            &mut output,
        )
        .unwrap();
        assert_eq!(output[0], 3);
        assert_eq!(output[1], 4);
        assert_eq!(u16::from_be_bytes([output[6], output[7]]), 1400);
        assert_eq!(internet_checksum(&output[..length]), 0);
    }
}