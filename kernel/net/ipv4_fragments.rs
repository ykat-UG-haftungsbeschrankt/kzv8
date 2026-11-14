/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use crate::kernel::error::Error;
use crate::kernel::net::wire::{internet_checksum, Ipv4Address, Ipv4Packet};

const MAX_DATAGRAMS: usize = 16;
const MAX_FRAGMENTS_PER_DATAGRAM: usize = 64;
const MAX_BUFFERED_BYTES: usize = 1024 * 1024;
const MAX_IPV4_PAYLOAD: usize = u16::MAX as usize - 20;
const FRAGMENT_TIMEOUT_NS: u64 = 30_000_000_000;

#[derive(Clone, Copy, Eq, PartialEq)]
struct FragmentKey {
    source: Ipv4Address,
    destination: Ipv4Address,
    identification: u16,
    protocol: u8,
}

struct FragmentPiece {
    offset: usize,
    //@FIXME: INDEXTYPE(bytes); contiguous fragment payload; Vec indexing is O(1), keep Vec.
    bytes: Vec<u8>,
}

struct DatagramFragments {
    key: FragmentKey,
    expires_at_ns: u64,
    //@FIXME: INDEXTYPE(header); optional contiguous header bytes; Vec indexing is O(1), keep Vec.
    header: Option<Vec<u8>>,
    //@FIXME: INDEXTYPE(pieces); offset-ordered fragment list capped at 64; O(n) scans are bounded, keep Vec.
    pieces: Vec<FragmentPiece>,
    final_length: Option<usize>,
    buffered_bytes: usize,
}

pub(crate) struct Ipv4FragmentReassembler {
    //@FIXME: INDEXTYPE(datagrams); key-searched reassembly list capped at 16; O(n) lookup/removal are bounded, keep Vec.
    datagrams: Vec<DatagramFragments>,
    buffered_bytes: usize,
}

impl Ipv4FragmentReassembler {
    pub(crate) const fn new() -> Self {
        Self {
            datagrams: Vec::new(),
            buffered_bytes: 0,
        }
    }

    pub(crate) fn insert(
        &mut self,
        packet: &Ipv4Packet<'_>,
        now_ns: u64,
    ) -> Result<Option<Vec<u8>>, Error> {
        self.expire(now_ns);
        let offset = usize::from(packet.fragment_offset)
            .checked_mul(8)
            .ok_or(Error::MsgSize)?;
        let end = offset.checked_add(packet.payload.len()).ok_or(Error::MsgSize)?;
        if packet.payload.is_empty()
            || packet.dont_fragment
            || packet.more_fragments && packet.payload.len() % 8 != 0
            || end > MAX_IPV4_PAYLOAD
        {
            return Ok(None);
        }

        let key = FragmentKey {
            source: packet.source,
            destination: packet.destination,
            identification: packet.identification,
            protocol: packet.protocol,
        };
        let existing_index = self.datagrams.iter().position(|datagram| datagram.key == key);
        if existing_index.is_none() && self.datagrams.len() >= MAX_DATAGRAMS {
            return Ok(None);
        }
        if let Some(index) = existing_index {
            let datagram = &self.datagrams[index];
            let invalid = datagram.pieces.len() >= MAX_FRAGMENTS_PER_DATAGRAM
                || datagram.header.is_some() && offset == 0
                || datagram.final_length.is_some_and(|length| end > length)
                || datagram.pieces.iter().any(|piece| {
                    let piece_end = piece.offset.saturating_add(piece.bytes.len());
                    offset < piece_end && piece.offset < end
                })
                || (!packet.more_fragments
                    && datagram.final_length.is_some_and(|length| length != end))
                || (!packet.more_fragments
                    && datagram.pieces.iter().any(|piece| {
                        piece.offset.saturating_add(piece.bytes.len()) > end
                    }));
            if invalid {
                self.remove_datagram(index);
                return Ok(None);
            }
        }

        let header_length = if offset == 0 { packet.header.len() } else { 0 };
        let added_bytes = header_length
            .checked_add(packet.payload.len())
            .ok_or(Error::NoMem)?;
        if self.buffered_bytes.saturating_add(added_bytes) > MAX_BUFFERED_BYTES {
            if let Some(index) = existing_index {
                self.remove_datagram(index);
            }
            return Ok(None);
        }

        let mut fragment_bytes = Vec::new();
        fragment_bytes
            .try_reserve_exact(packet.payload.len())
            .map_err(|_| Error::NoMem)?;
        fragment_bytes.extend_from_slice(packet.payload);
        let header = if offset == 0 {
            let mut header = Vec::new();
            header
                .try_reserve_exact(packet.header.len())
                .map_err(|_| Error::NoMem)?;
            header.extend_from_slice(packet.header);
            Some(header)
        } else {
            None
        };

        let index = if let Some(index) = existing_index {
            index
        } else {
            let mut pieces = Vec::new();
            pieces.try_reserve(1).map_err(|_| Error::NoMem)?;
            self.datagrams.try_reserve(1).map_err(|_| Error::NoMem)?;
            self.datagrams.push(DatagramFragments {
                key,
                expires_at_ns: now_ns.saturating_add(FRAGMENT_TIMEOUT_NS),
                header: None,
                pieces,
                final_length: None,
                buffered_bytes: 0,
            });
            self.datagrams.len() - 1
        };
        let datagram = &mut self.datagrams[index];
        datagram.pieces.try_reserve(1).map_err(|_| Error::NoMem)?;
        datagram.pieces.push(FragmentPiece {
            offset,
            bytes: fragment_bytes,
        });
        if let Some(header) = header {
            datagram.header = Some(header);
        }
        if !packet.more_fragments {
            datagram.final_length = Some(end);
        }
        datagram.buffered_bytes += added_bytes;
        self.buffered_bytes += added_bytes;

        self.datagrams[index].pieces.sort_unstable_by_key(|piece| piece.offset);
        let datagram = &self.datagrams[index];
        let (Some(header), Some(final_length)) = (&datagram.header, datagram.final_length) else {
            return Ok(None);
        };
        let mut covered = 0usize;
        for piece in &datagram.pieces {
            if piece.offset != covered {
                return Ok(None);
            }
            covered = covered.checked_add(piece.bytes.len()).ok_or(Error::MsgSize)?;
        }
        if covered != final_length {
            return Ok(None);
        }

        let total_length = header.len().checked_add(final_length).ok_or(Error::MsgSize)?;
        let Ok(total_length_u16) = u16::try_from(total_length) else {
            self.remove_datagram(index);
            return Ok(None);
        };
        let mut reassembled = Vec::new();
        reassembled
            .try_reserve_exact(total_length)
            .map_err(|_| Error::NoMem)?;
        reassembled.extend_from_slice(header);
        for piece in &datagram.pieces {
            reassembled.extend_from_slice(&piece.bytes);
        }
        reassembled[2..4].copy_from_slice(&total_length_u16.to_be_bytes());
        let original_fragment = u16::from_be_bytes([reassembled[6], reassembled[7]]);
        reassembled[6..8].copy_from_slice(&(original_fragment & 0x4000).to_be_bytes());
        reassembled[10..12].fill(0);
        let checksum = internet_checksum(&reassembled[..header.len()]);
        reassembled[10..12].copy_from_slice(&checksum.to_be_bytes());
        self.remove_datagram(index);
        Ok(Some(reassembled))
    }

    pub(crate) fn expire(&mut self, now_ns: u64) {
        let mut index = 0;
        while index < self.datagrams.len() {
            if now_ns >= self.datagrams[index].expires_at_ns {
                self.remove_datagram(index);
            } else {
                index += 1;
            }
        }
    }

    fn remove_datagram(&mut self, index: usize) {
        let datagram = self.datagrams.remove(index);
        self.buffered_bytes = self.buffered_bytes.saturating_sub(datagram.buffered_bytes);
    }
}
