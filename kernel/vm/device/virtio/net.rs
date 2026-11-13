/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::vm::device::virtio::mmio::{QueueResult, VirtioMmio};
use crate::kernel::vm::device::virtio::queue::GuestVirtQueue;
use crate::kernel::vm::device::GuestMemory;
use crate::kernel::net::wire::EthernetFrame;
use crate::kernel::vfs::plugin::tun::{self, TunTapMode};

pub(crate) const MMIO_BASE: u64 = 0x1000_3000;
const DEVICE_ID: u32 = 1;
const VENDOR_ID: u32 = 0x4b5a;
const MAX_FRAME_SIZE: usize = 1514;
const HEADER_SIZE: usize = 12;
const MAX_HUB_PEERS: usize = 256;
const MAX_PEER_QUEUE_BYTES: usize = 256 * 1024;
const MAX_HOST_QUEUE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SWITCH_QUEUE_BYTES: usize = 16 * 1024 * 1024;
const FEATURE_MTU: u32 = 1 << 3;
const FEATURE_MAC: u32 = 1 << 5;
const FEATURE_MRG_RXBUF: u32 = 1 << 15;

static NETWORK_SWITCH: IrqSpinLock<NetworkSwitch> = IrqSpinLock::new(NetworkSwitch::new());

struct Peer {
    active: bool,
    mac: [u8; 6],
    frames: VecDeque<Vec<u8>>,
    queued_bytes: usize,
}

impl Peer {
    fn new() -> Self {
        Self {
            active: false,
            mac: [0; 6],
            frames: VecDeque::new(),
            queued_bytes: 0,
        }
    }
}

struct NetworkSwitch {
    peers: Vec<Peer>,
    host_active: bool,
    host_mac: [u8; 6],
    host_stack_frames: VecDeque<Vec<u8>>,
    host_stack_bytes: usize,
    host_transmit_frames: VecDeque<Vec<u8>>,
    host_transmit_bytes: usize,
    queued_frame_bytes: usize,
    dropped_frames: u64,
}

impl NetworkSwitch {
    const fn new() -> Self {
        Self {
            peers: Vec::new(),
            host_active: false,
            host_mac: [0; 6],
            host_stack_frames: VecDeque::new(),
            host_stack_bytes: 0,
            host_transmit_frames: VecDeque::new(),
            host_transmit_bytes: 0,
            queued_frame_bytes: 0,
            dropped_frames: 0,
        }
    }

    fn register_host(&mut self, mac: [u8; 6]) {
        self.host_active = true;
        self.host_mac = mac;
    }

    fn register(&mut self) -> Option<(usize, [u8; 6])> {
        let index = if let Some(index) = self.peers.iter().position(|peer| !peer.active) {
            index
        } else {
            if self.peers.len() >= MAX_HUB_PEERS {
                return None;
            }
            self.peers.try_reserve(1).ok()?;
            self.peers.push(Peer::new());
            self.peers.len() - 1
        };
        let identifier = u16::try_from(index + 1).ok()?;
        let mac = [0x02, 0x4b, 0x5a, 0, (identifier >> 8) as u8, identifier as u8];
        self.peers[index].active = true;
        self.peers[index].mac = mac;
        Some((index, mac))
    }

    fn unregister(&mut self, peer_id: usize) {
        let Some(peer) = self.peers.get(peer_id) else {
            return;
        };
        let queued_bytes = peer.queued_bytes;
        let queued_frames = peer.frames.len() as u64;
        self.queued_frame_bytes = self.queued_frame_bytes.saturating_sub(queued_bytes);
        self.dropped_frames = self.dropped_frames.saturating_add(queued_frames);
        self.peers[peer_id] = Peer::new();
    }

    fn transmit(&mut self, source_id: usize, bytes: &[u8]) {
        if bytes.len() > MAX_FRAME_SIZE {
            self.record_dropped_frame();
            return;
        }
        let Ok(frame) = EthernetFrame::parse(bytes) else {
            return;
        };
        let source = source_id;
        if source >= self.peers.len() || !self.peers[source].active {
            return;
        }
        let destination = frame.destination.0;
        let multicast = destination[0] & 1 != 0;
        let destination_peer_exists = self
            .peers
            .iter()
            .any(|peer| peer.active && peer.mac == destination);
        let destination_tap_exists = self.tap_mac_exists(destination);
        let destination_host = self.host_active && self.host_mac == destination;
        self.forward_to_peers(
            Some(source),
            destination,
            bytes,
            !destination_peer_exists && !destination_tap_exists && !destination_host,
        );
        self.forward_to_taps(destination, None, bytes);
        if multicast || destination_host {
            self.enqueue_host_stack(bytes);
        }
        if multicast
            || (!destination_peer_exists && !destination_tap_exists && !destination_host)
        {
            self.enqueue_host_transmit(bytes);
        }
        self.drain_tap_transmissions();
    }

    fn forward_tap_frame(&mut self, source_mac: [u8; 6], bytes: &[u8]) {
        if bytes.len() > MAX_FRAME_SIZE {
            self.record_dropped_frame();
            return;
        }
        let Ok(frame) = EthernetFrame::parse(bytes) else {
            return;
        };
        let destination = frame.destination.0;
        let multicast = destination[0] & 1 != 0;
        let destination_peer_exists = self
            .peers
            .iter()
            .any(|peer| peer.active && peer.mac == destination);
        let destination_tap_exists = self.tap_mac_exists(destination);
        let destination_host = self.host_active && self.host_mac == destination;
        self.forward_to_peers(
            None,
            destination,
            bytes,
            !destination_peer_exists && !destination_tap_exists && !destination_host,
        );
        self.forward_to_taps(destination, Some(source_mac), bytes);
        if multicast || destination_host {
            self.enqueue_host_stack(bytes);
        }
        if multicast
            || (!destination_peer_exists && !destination_tap_exists && !destination_host)
        {
            self.enqueue_host_transmit(bytes);
        }
    }

    fn forward_to_peers(
        &mut self,
        source: Option<usize>,
        destination: [u8; 6],
        bytes: &[u8],
        flood_unknown_unicast: bool,
    ) {
        let multicast = destination[0] & 1 != 0;
        let destination_peer = self
            .peers
            .iter()
            .position(|peer| peer.active && peer.mac == destination);
        for index in 0..self.peers.len() {
            if source == Some(index) || !self.peers[index].active {
                continue;
            }
            let deliver = if multicast {
                true
            } else if let Some(destination_peer) = destination_peer {
                index == destination_peer
            } else {
                flood_unknown_unicast
            };
            if !deliver {
                continue;
            }
            self.enqueue_peer_frame(index, bytes);
        }
    }

    fn forward_to_taps(
        &mut self,
        destination: [u8; 6],
        source_mac: Option<[u8; 6]>,
        bytes: &[u8],
    ) {
        let Ok(taps) = tun::open_devices() else {
            return;
        };
        let destination_peer_exists = self
            .peers
            .iter()
            .any(|peer| peer.active && peer.mac == destination);
        let destination_tap_exists = taps.iter().any(|tap| {
            tap.mode() == TunTapMode::Tap && tap.mac_address() == destination
        });
        let multicast = destination[0] & 1 != 0;
        for tap in taps {
            if !tap.is_configured()
                || tap.mode() != TunTapMode::Tap
                || source_mac == Some(tap.mac_address())
            {
                continue;
            }
            let deliver = multicast
                || tap.mac_address() == destination
                || (!destination_peer_exists && !destination_tap_exists);
            if deliver && tap.inject_packet(bytes).is_err() {
                self.dropped_frames = self.dropped_frames.saturating_add(1);
            }
        }
    }

    fn drain_tap_transmissions(&mut self) {
        let Ok(taps) = tun::open_devices() else {
            return;
        };
        for tap in taps {
            if !tap.is_configured() || tap.mode() != TunTapMode::Tap {
                continue;
            }
            while let Ok(frame) = tap.dequeue_transmitted_packet() {
                self.forward_tap_frame(tap.mac_address(), &frame);
            }
        }
    }

    fn host_ingress(&mut self, bytes: &[u8]) {
        if bytes.len() > MAX_FRAME_SIZE {
            self.record_dropped_frame();
            return;
        }
        let Ok(frame) = EthernetFrame::parse(bytes) else {
            return;
        };
        if self.host_active && frame.destination.0 == self.host_mac {
            return;
        }
        let destination = frame.destination.0;
        let peer_exists = self
            .peers
            .iter()
            .any(|peer| peer.active && peer.mac == destination);
        let tap_exists = self.tap_mac_exists(destination);
        self.forward_to_peers(None, destination, bytes, !peer_exists && !tap_exists);
        self.forward_to_taps(destination, None, bytes);
    }

    fn host_transmit(&mut self, bytes: &[u8]) {
        if bytes.len() > MAX_FRAME_SIZE {
            self.record_dropped_frame();
            return;
        }
        let Ok(frame) = EthernetFrame::parse(bytes) else {
            return;
        };
        let destination = frame.destination.0;
        if self.host_active && destination == self.host_mac {
            return;
        }
        let multicast = destination[0] & 1 != 0;
        let destination_peer_exists = self
            .peers
            .iter()
            .any(|peer| peer.active && peer.mac == destination);
        let destination_tap_exists = self.tap_mac_exists(destination);
        if multicast {
            self.forward_to_peers(None, destination, bytes, true);
            self.forward_to_taps(destination, None, bytes);
            self.enqueue_host_transmit(bytes);
        } else if destination_peer_exists {
            self.forward_to_peers(None, destination, bytes, false);
        } else if destination_tap_exists {
            self.forward_to_taps(destination, None, bytes);
        } else {
            self.enqueue_host_transmit(bytes);
        }
        self.drain_tap_transmissions();
    }

    fn tap_mac_exists(&self, mac: [u8; 6]) -> bool {
        tun::open_devices().is_ok_and(|devices| {
            devices
                .iter()
                .any(|device| device.mode() == TunTapMode::Tap && device.mac_address() == mac)
        })
    }

    fn enqueue_host_stack(&mut self, bytes: &[u8]) {
        self.enqueue_host_frame(true, bytes);
    }

    fn enqueue_host_transmit(&mut self, bytes: &[u8]) {
        self.enqueue_host_frame(false, bytes);
    }

    fn receive_host_stack(&mut self, output: &mut [u8]) -> Option<usize> {
        if self
            .host_stack_frames
            .front()
            .is_some_and(|frame| frame.len() > output.len())
        {
            let frame = self.host_stack_frames.pop_front()?;
            let length = frame.len();
            self.host_stack_bytes = self.host_stack_bytes.saturating_sub(length);
            self.queued_frame_bytes = self.queued_frame_bytes.saturating_sub(length);
            self.record_dropped_frame();
            return Some(0);
        }
        let length = dequeue_frame(&mut self.host_stack_frames, output)?;
        self.host_stack_bytes = self.host_stack_bytes.saturating_sub(length);
        self.queued_frame_bytes = self.queued_frame_bytes.saturating_sub(length);
        Some(length)
    }

    fn peek_host_transmit(&mut self, output: &mut [u8]) -> Option<usize> {
        if self
            .host_transmit_frames
            .front()
            .is_some_and(|frame| frame.len() > output.len())
        {
            let frame = self.host_transmit_frames.pop_front()?;
            let length = frame.len();
            self.host_transmit_bytes = self.host_transmit_bytes.saturating_sub(length);
            self.queued_frame_bytes = self.queued_frame_bytes.saturating_sub(length);
            self.record_dropped_frame();
            return Some(0);
        }
        let frame = self.host_transmit_frames.front()?;
        let length = frame.len();
        output[..length].copy_from_slice(frame);
        Some(length)
    }

    fn consume_host_transmit(&mut self, expected_length: usize, dropped: bool) -> bool {
        let Some(frame) = self.host_transmit_frames.front() else {
            return false;
        };
        if frame.len() != expected_length {
            return false;
        }
        let Some(frame) = self.host_transmit_frames.pop_front() else {
            return false;
        };
        let length = frame.len();
        self.host_transmit_bytes = self.host_transmit_bytes.saturating_sub(length);
        self.queued_frame_bytes = self.queued_frame_bytes.saturating_sub(length);
        if dropped {
            self.dropped_frames = self.dropped_frames.saturating_add(1);
        }
        true
    }

    fn enqueue_peer_frame(&mut self, peer_id: usize, bytes: &[u8]) {
        if bytes.is_empty()
            || bytes.len() > MAX_FRAME_SIZE
            || self.queued_frame_bytes.saturating_add(bytes.len()) > MAX_SWITCH_QUEUE_BYTES
        {
            self.dropped_frames = self.dropped_frames.saturating_add(1);
            return;
        }
        let mut frame = Vec::new();
        if frame.try_reserve_exact(bytes.len()).is_err() {
            self.dropped_frames = self.dropped_frames.saturating_add(1);
            return;
        }
        frame.extend_from_slice(bytes);
        let enqueued = if let Some(peer) = self.peers.get_mut(peer_id) {
            if peer.active
                && peer.queued_bytes.saturating_add(bytes.len()) <= MAX_PEER_QUEUE_BYTES
                && peer.frames.try_reserve(1).is_ok()
            {
                peer.frames.push_back(frame);
                peer.queued_bytes += bytes.len();
                true
            } else {
                false
            }
        } else {
            false
        };
        if enqueued {
            self.queued_frame_bytes += bytes.len();
        } else {
            self.dropped_frames = self.dropped_frames.saturating_add(1);
        }
    }

    fn enqueue_host_frame(&mut self, to_stack: bool, bytes: &[u8]) {
        if bytes.is_empty()
            || bytes.len() > MAX_FRAME_SIZE
            || self.queued_frame_bytes.saturating_add(bytes.len()) > MAX_SWITCH_QUEUE_BYTES
        {
            self.dropped_frames = self.dropped_frames.saturating_add(1);
            return;
        }
        let mut frame = Vec::new();
        if frame.try_reserve_exact(bytes.len()).is_err() {
            self.dropped_frames = self.dropped_frames.saturating_add(1);
            return;
        }
        frame.extend_from_slice(bytes);
        let enqueued = if to_stack {
            if self.host_stack_bytes.saturating_add(bytes.len()) > MAX_HOST_QUEUE_BYTES
                || self.host_stack_frames.try_reserve(1).is_err()
            {
                false
            } else {
                self.host_stack_frames.push_back(frame);
                self.host_stack_bytes += bytes.len();
                true
            }
        } else if self.host_transmit_bytes.saturating_add(bytes.len()) > MAX_HOST_QUEUE_BYTES
            || self.host_transmit_frames.try_reserve(1).is_err()
        {
            false
        } else {
            self.host_transmit_frames.push_back(frame);
            self.host_transmit_bytes += bytes.len();
            true
        };
        if enqueued {
            self.queued_frame_bytes += bytes.len();
        } else {
            self.dropped_frames = self.dropped_frames.saturating_add(1);
        }
    }

    fn receive(&mut self, peer_id: usize) -> Option<Vec<u8>> {
        let peer = self.peers.get_mut(peer_id)?;
        if !peer.active {
            return None;
        }
        let frame = peer.frames.pop_front()?;
        let length = frame.len();
        peer.queued_bytes = peer.queued_bytes.saturating_sub(length);
        self.queued_frame_bytes = self.queued_frame_bytes.saturating_sub(length);
        Some(frame)
    }

    fn dropped_frame_count(&self) -> u64 {
        self.dropped_frames
    }

    fn record_dropped_frame(&mut self) {
        self.dropped_frames = self.dropped_frames.saturating_add(1);
    }
}

fn dequeue_frame(frames: &mut VecDeque<Vec<u8>>, output: &mut [u8]) -> Option<usize> {
    let frame = frames.front()?;
    if output.len() < frame.len() {
        return None;
    }
    let frame = frames.pop_front()?;
    let frame_length = frame.len();
    output[..frame_length].copy_from_slice(&frame);
    Some(frame_length)
}

pub(crate) fn register_host_interface(mac: [u8; 6]) {
    NETWORK_SWITCH.lock().register_host(mac);
}

pub(crate) fn host_ingress_frame(bytes: &[u8]) {
    NETWORK_SWITCH.lock().host_ingress(bytes);
}

pub(crate) fn host_transmit_frame(bytes: &[u8]) {
    NETWORK_SWITCH.lock().host_transmit(bytes);
}

pub(crate) fn receive_host_stack_frame(output: &mut [u8]) -> Option<usize> {
    NETWORK_SWITCH.lock().receive_host_stack(output)
}

pub(crate) fn peek_host_transmit_frame(output: &mut [u8]) -> Option<usize> {
    NETWORK_SWITCH.lock().peek_host_transmit(output)
}

pub(crate) fn consume_host_transmit_frame(length: usize) -> bool {
    NETWORK_SWITCH.lock().consume_host_transmit(length, false)
}

pub(crate) fn drop_host_transmit_frame(length: usize) -> bool {
    NETWORK_SWITCH.lock().consume_host_transmit(length, true)
}

#[allow(dead_code)]
pub(crate) fn dropped_frame_count() -> u64 {
    NETWORK_SWITCH.lock().dropped_frame_count()
}

pub(crate) struct VirtioNet {
    mmio: VirtioMmio,
    peer_id: usize,
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
                256,
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
        poll_tap_devices();
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
        peer_id: usize,
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
        peer_id: usize,
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
            let total_length = HEADER_SIZE + frame.len();
            if capacity < total_length {
                NETWORK_SWITCH.lock().record_dropped_frame();
                if !queue.complete(memory, &chain, 0) {
                    return QueueResult::Invalid;
                }
                completed = true;
                continue;
            }
            let mut packet = [0; HEADER_SIZE + MAX_FRAME_SIZE];
            packet[10..12].copy_from_slice(&1u16.to_le_bytes());
            packet[HEADER_SIZE..total_length].copy_from_slice(&frame);
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
        peer_id: usize,
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

pub(crate) fn poll_tap_devices() {
    NETWORK_SWITCH.lock().drain_tap_transmissions();
}

impl Drop for VirtioNet {
    fn drop(&mut self) {
        NETWORK_SWITCH.lock().unregister(self.peer_id);
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/vm/device/virtio/net.rs"]
mod tests;
