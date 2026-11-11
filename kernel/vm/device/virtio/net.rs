/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::vm::device::virtio::mmio::{QueueResult, VirtioMmio};
use crate::kernel::vm::device::virtio::queue::GuestVirtQueue;
use crate::kernel::vm::device::GuestMemory;

pub(crate) const MMIO_BASE: u64 = 0x1000_3000;
const DEVICE_ID: u32 = 1;
const VENDOR_ID: u32 = 0x4b5a;
const MAX_FRAME_SIZE: usize = 1514;
const HEADER_SIZE: usize = 12;
const HUB_PEER_COUNT: usize = 8;
const RX_RING_SIZE: usize = 8;
const FEATURE_MTU: u32 = 1 << 3;
const FEATURE_MAC: u32 = 1 << 5;
const FEATURE_MRG_RXBUF: u32 = 1 << 15;

static NETWORK_SWITCH: IrqSpinLock<NetworkSwitch> = IrqSpinLock::new(NetworkSwitch::new());

#[derive(Clone, Copy)]
struct Frame {
    length: usize,
    bytes: [u8; MAX_FRAME_SIZE],
}

impl Frame {
    const EMPTY: Self = Self {
        length: 0,
        bytes: [0; MAX_FRAME_SIZE],
    };
}

#[derive(Clone, Copy)]
struct Peer {
    active: bool,
    mac: [u8; 6],
    frames: [Frame; RX_RING_SIZE],
    head: usize,
    length: usize,
}

impl Peer {
    const EMPTY: Self = Self {
        active: false,
        mac: [0; 6],
        frames: [Frame::EMPTY; RX_RING_SIZE],
        head: 0,
        length: 0,
    };
}

struct NetworkSwitch {
    peers: [Peer; HUB_PEER_COUNT],
}

impl NetworkSwitch {
    const fn new() -> Self {
        Self {
            peers: [Peer::EMPTY; HUB_PEER_COUNT],
        }
    }

    fn register(&mut self) -> Option<(u8, [u8; 6])> {
        let index = self.peers.iter().position(|peer| !peer.active)?;
        let mac = [0x02, 0x4b, 0x5a, 0x00, 0x00, index as u8 + 1];
        self.peers[index] = Peer {
            active: true,
            mac,
            ..Peer::EMPTY
        };
        Some((index as u8, mac))
    }

    fn unregister(&mut self, peer_id: u8) {
        if let Some(peer) = self.peers.get_mut(usize::from(peer_id)) {
            *peer = Peer::EMPTY;
        }
    }

    fn transmit(&mut self, source_id: u8, bytes: &[u8]) {
        if bytes.len() < 14 || bytes.len() > MAX_FRAME_SIZE {
            return;
        }
        let source = usize::from(source_id);
        if source >= self.peers.len() || !self.peers[source].active {
            return;
        }
        let destination: [u8; 6] = match bytes.get(..6).and_then(|mac| mac.try_into().ok()) {
            Some(mac) => mac,
            None => return,
        };
        let multicast = destination[0] & 1 != 0;
        let destination_peer = self
            .peers
            .iter()
            .position(|peer| peer.active && peer.mac == destination);
        for index in 0..self.peers.len() {
            if index == source || !self.peers[index].active {
                continue;
            }
            let deliver = if multicast {
                true
            } else if let Some(destination_peer) = destination_peer {
                index == destination_peer
            } else {
                true
            };
            if !deliver {
                continue;
            }
            let peer = &mut self.peers[index];
            if peer.length == RX_RING_SIZE {
                continue;
            }
            let tail = (peer.head + peer.length) % RX_RING_SIZE;
            let frame = &mut peer.frames[tail];
            frame.bytes[..bytes.len()].copy_from_slice(bytes);
            frame.length = bytes.len();
            peer.length += 1;
        }
    }

    fn receive(&mut self, peer_id: u8) -> Option<Frame> {
        let peer = self.peers.get_mut(usize::from(peer_id))?;
        if !peer.active || peer.length == 0 {
            return None;
        }
        let frame = peer.frames[peer.head];
        peer.head = (peer.head + 1) % RX_RING_SIZE;
        peer.length -= 1;
        Some(frame)
    }
}

pub(crate) struct VirtioNet {
    mmio: VirtioMmio,
    peer_id: u8,
}

impl VirtioNet {
    pub(crate) fn new() -> Result<Self, crate::kernel::arch::VirtualizationError> {
        let (peer_id, mac) = NETWORK_SWITCH
            .lock()
            .register()
            .ok_or(crate::kernel::arch::VirtualizationError::ResourceExhausted)?;
        let mut config = [0; 16];
        config[0] = u32::from_le_bytes([mac[0], mac[1], mac[2], mac[3]]);
        config[1] = u32::from_le_bytes([mac[4], mac[5], 1, 0]);
        config[2] = 1 | (1500 << 16);
        Ok(Self {
            mmio: VirtioMmio::new(
                MMIO_BASE,
                DEVICE_ID,
                VENDOR_ID,
                [FEATURE_MTU | FEATURE_MAC | FEATURE_MRG_RXBUF, 1],
                config,
                [0; 16],
                2,
            ),
            peer_id,
        })
    }

    pub(crate) const fn contains(address: u64) -> bool {
        address >= MMIO_BASE && address < MMIO_BASE + 0x1000
    }

    pub(crate) fn access(
        &mut self,
        memory: &mut GuestMemory<'_>,
        address: u64,
        width: u8,
        write: bool,
        value: u64,
    ) -> Option<u64> {
        if !Self::contains(address) {
            return None;
        }
        let peer_id = self.peer_id;
        let mmio = &mut self.mmio;
        let receive_result = mmio
            .queue_mut(0)
            .filter(|queue| queue.is_ready())
            .map(|queue| Self::receive_available(peer_id, queue, memory));
        if matches!(receive_result, Some(QueueResult::Complete)) {
            mmio.signal_used_buffer();
        }
        mmio.access(memory, address, width, write, value, |queue_index, queue, memory| {
            Self::notify_queue(peer_id, queue_index, queue, memory)
        })
    }

    fn notify_queue(
        peer_id: u8,
        queue_index: usize,
        queue: &mut GuestVirtQueue,
        memory: &mut GuestMemory<'_>,
    ) -> QueueResult {
        match queue_index {
            0 => Self::receive_available(peer_id, queue, memory),
            1 => Self::transmit_available(peer_id, queue, memory),
            _ => QueueResult::Invalid,
        }
    }

    fn receive_available(
        peer_id: u8,
        queue: &mut GuestVirtQueue,
        memory: &mut GuestMemory<'_>,
    ) -> QueueResult {
        let mut completed = false;
        loop {
            let chain = match queue.available_chain(memory) {
                Ok(Some(chain)) => chain,
                Ok(None) => break,
                Err(()) => return QueueResult::Invalid,
            };
            let Some(capacity) = chain.writable_capacity() else {
                return QueueResult::Invalid;
            };
            let frame = NETWORK_SWITCH.lock().receive(peer_id);
            let Some(frame) = frame else {
                break;
            };
            let total_length = HEADER_SIZE + frame.length;
            if capacity < total_length {
                if !queue.complete(memory, &chain, 0) {
                    return QueueResult::Invalid;
                }
                completed = true;
                continue;
            }
            let mut packet = [0; HEADER_SIZE + MAX_FRAME_SIZE];
            packet[10..12].copy_from_slice(&1u16.to_le_bytes());
            packet[HEADER_SIZE..total_length].copy_from_slice(&frame.bytes[..frame.length]);
            let Some(written) = chain.write_device_data(memory, &packet[..total_length]) else {
                return QueueResult::Invalid;
            };
            if !queue.complete(memory, &chain, written) {
                return QueueResult::Invalid;
            }
            completed = true;
        }
        if completed {
            QueueResult::Complete
        } else {
            QueueResult::Pending
        }
    }

    fn transmit_available(
        peer_id: u8,
        queue: &mut GuestVirtQueue,
        memory: &mut GuestMemory<'_>,
    ) -> QueueResult {
        let mut completed = false;
        loop {
            let chain = match queue.available_chain(memory) {
                Ok(Some(chain)) => chain,
                Ok(None) => break,
                Err(()) => return QueueResult::Invalid,
            };
            let Some(capacity) = chain.readable_capacity() else {
                return QueueResult::Invalid;
            };
            if !(HEADER_SIZE + 14..=HEADER_SIZE + MAX_FRAME_SIZE).contains(&capacity) {
                return QueueResult::Invalid;
            }
            let mut packet = [0; HEADER_SIZE + MAX_FRAME_SIZE];
            let Some(length) = chain.read_device_data(memory, &mut packet) else {
                return QueueResult::Invalid;
            };
            if packet[..HEADER_SIZE].iter().any(|byte| *byte != 0) {
                return QueueResult::Invalid;
            }
            NETWORK_SWITCH
                .lock()
                .transmit(peer_id, &packet[HEADER_SIZE..length]);
            if !queue.complete(memory, &chain, length) {
                return QueueResult::Invalid;
            }
            completed = true;
        }
        if completed {
            QueueResult::Complete
        } else {
            QueueResult::Pending
        }
    }
}

impl Drop for VirtioNet {
    fn drop(&mut self) {
        NETWORK_SWITCH.lock().unregister(self.peer_id);
    }
}
