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