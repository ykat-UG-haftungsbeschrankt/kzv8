/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::net::wire::{Ipv4Address, MacAddress, ETHER_TYPE_IPV4};

pub(crate) const ARP_PACKET_LENGTH: usize = 28;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArpOperation {
    Request,
    Reply,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArpError {
    Truncated,
    UnsupportedAddressFormat,
    UnsupportedOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ArpPacket {
    pub(crate) operation: ArpOperation,
    pub(crate) sender_mac: MacAddress,
    pub(crate) sender_ipv4: Ipv4Address,
    pub(crate) target_mac: MacAddress,
    pub(crate) target_ipv4: Ipv4Address,
}

impl ArpPacket {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, ArpError> {
        if bytes.len() < ARP_PACKET_LENGTH {
            return Err(ArpError::Truncated);
        }
        if u16::from_be_bytes([bytes[0], bytes[1]]) != 1
            || u16::from_be_bytes([bytes[2], bytes[3]]) != ETHER_TYPE_IPV4
            || bytes[4] != 6
            || bytes[5] != 4
        {
            return Err(ArpError::UnsupportedAddressFormat);
        }
        let operation = match u16::from_be_bytes([bytes[6], bytes[7]]) {
            1 => ArpOperation::Request,
            2 => ArpOperation::Reply,
            _ => return Err(ArpError::UnsupportedOperation),
        };
        let mut sender_mac = [0; 6];
        sender_mac.copy_from_slice(&bytes[8..14]);
        let mut sender_ipv4 = [0; 4];
        sender_ipv4.copy_from_slice(&bytes[14..18]);
        let mut target_mac = [0; 6];
        target_mac.copy_from_slice(&bytes[18..24]);
        let mut target_ipv4 = [0; 4];
        target_ipv4.copy_from_slice(&bytes[24..28]);
        Ok(Self {
            operation,
            sender_mac: MacAddress(sender_mac),
            sender_ipv4: Ipv4Address(sender_ipv4),
            target_mac: MacAddress(target_mac),
            target_ipv4: Ipv4Address(target_ipv4),
        })
    }

    pub(crate) fn write(self, output: &mut [u8]) -> Result<(), ArpError> {
        if output.len() < ARP_PACKET_LENGTH {
            return Err(ArpError::Truncated);
        }
        output[..ARP_PACKET_LENGTH].fill(0);
        output[0..2].copy_from_slice(&1u16.to_be_bytes());
        output[2..4].copy_from_slice(&ETHER_TYPE_IPV4.to_be_bytes());
        output[4] = 6;
        output[5] = 4;
        let operation = match self.operation {
            ArpOperation::Request => 1u16,
            ArpOperation::Reply => 2u16,
        };
        output[6..8].copy_from_slice(&operation.to_be_bytes());
        output[8..14].copy_from_slice(&self.sender_mac.0);
        output[14..18].copy_from_slice(&self.sender_ipv4.0);
        output[18..24].copy_from_slice(&self.target_mac.0);
        output[24..28].copy_from_slice(&self.target_ipv4.0);
        Ok(())
    }
}