/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::kernel::error::Error;
use crate::kernel::net::wire::Ipv4Address;
use crate::kernel::scheduler::sync::IrqSpinLock;

use super::super::{FilePlugin, OpenMode, VfsFuture, VfsResult};

const MAX_TUN_PACKET_SIZE: usize = 65_535;
const MAX_TAP_FRAME_SIZE: usize = 1514;
const MAX_QUEUED_PACKETS: usize = 64;
static NEXT_TAP_MAC_ID: AtomicU32 = AtomicU32::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TunTapMode {
    Tun,
    Tap,
}

struct PacketQueues {
    //@FIXME: INDEXTYPE(to_userspace); FIFO VecDeque of packet Vec byte buffers; queue ends are O(1), keep VecDeque and Vec payloads.
    to_userspace: VecDeque<Vec<u8>>,
    //@FIXME: INDEXTYPE(to_kernel); FIFO VecDeque of packet Vec byte buffers; queue ends are O(1), keep VecDeque and Vec payloads.
    to_kernel: VecDeque<Vec<u8>>,
    mode: TunTapMode,
    configured: bool,
    ipv4_address: Option<Ipv4Address>,
    ipv4_prefix_length: u8,
    up: bool,
}

pub(crate) struct TunDevice {
    packets: IrqSpinLock<PacketQueues>,
    mac_address: [u8; 6],
}

//@FIXME: INDEXTYPE(OPEN_TUN_DEVICES); weak registry is appended, swept, and enumerated; keep Vec.
static OPEN_TUN_DEVICES: IrqSpinLock<Vec<Weak<TunDevice>>> =
    IrqSpinLock::new(Vec::new());

impl TunDevice {
    pub(crate) fn new() -> VfsResult<Arc<Self>> {
        let device = Arc::new(Self {
            packets: IrqSpinLock::new(PacketQueues {
                to_userspace: VecDeque::new(),
                to_kernel: VecDeque::new(),
                mode: TunTapMode::Tun,
                configured: false,
                ipv4_address: None,
                ipv4_prefix_length: 24,
                up: false,
            }),
            mac_address: {
                let id = NEXT_TAP_MAC_ID.fetch_add(1, Ordering::Relaxed) & 0x00ff_ffff;
                [0x02, 0x4b, 0x5a, (id >> 16) as u8, (id >> 8) as u8, id as u8]
            },
        });
        let mut devices = OPEN_TUN_DEVICES.lock();
        devices.retain(|registered| registered.strong_count() > 0);
        devices.try_reserve(1).map_err(|_| Error::NoMem)?;
        devices.push(Arc::downgrade(&device));
        Ok(device)
    }

    pub(crate) fn configure(&self, mode: TunTapMode) -> VfsResult<()> {
        let mut packets = self.packets.lock();
        if !packets.to_userspace.is_empty() || !packets.to_kernel.is_empty() {
            return Err(Error::Busy);
        }
        if packets.configured {
            return Err(Error::Busy);
        }
        packets.mode = mode;
        packets.configured = true;
        Ok(())
    }

    pub(crate) fn mode(&self) -> TunTapMode {
        self.packets.lock().mode
    }

    pub(crate) fn is_configured(&self) -> bool {
        self.packets.lock().configured
    }

    pub(crate) fn ipv4_configuration(&self) -> Option<(Ipv4Address, u8)> {
        let packets = self.packets.lock();
        packets
            .ipv4_address
            .map(|address| (address, packets.ipv4_prefix_length))
    }

    pub(crate) fn set_ipv4_address(&self, address: Ipv4Address) {
        self.packets.lock().ipv4_address = Some(address);
    }

    pub(crate) fn set_ipv4_prefix_length(&self, prefix_length: u8) {
        self.packets.lock().ipv4_prefix_length = prefix_length;
    }

    pub(crate) fn is_up(&self) -> bool {
        self.packets.lock().up
    }

    pub(crate) fn set_up(&self, up: bool) {
        self.packets.lock().up = up;
    }

    pub(crate) const fn mac_address(&self) -> [u8; 6] {
        self.mac_address
    }

    pub(crate) fn read_packet(&self, buffer: &mut [u8]) -> VfsResult<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let mut packets = self.packets.lock();
        let packet = packets.to_userspace.front().ok_or(Error::Again)?;
        if buffer.len() < packet.len() {
            return Err(Error::MsgSize);
        }
        let length = packet.len();
        buffer[..length].copy_from_slice(packet);
        packets.to_userspace.pop_front();
        Ok(length)
    }

    pub(crate) fn write_packet(&self, packet: &[u8]) -> VfsResult<usize> {
        let mut packets = self.packets.lock();
        let maximum_length = match packets.mode {
            TunTapMode::Tun => MAX_TUN_PACKET_SIZE,
            TunTapMode::Tap => MAX_TAP_FRAME_SIZE,
        };
        if packet.len() > maximum_length {
            return Err(Error::MsgSize);
        }
        queue_packet(&mut packets.to_kernel, packet)?;
        Ok(packet.len())
    }

    #[allow(dead_code)]
    pub(crate) fn inject_packet(&self, packet: &[u8]) -> VfsResult<()> {
        let mut packets = self.packets.lock();
        let maximum_length = match packets.mode {
            TunTapMode::Tun => MAX_TUN_PACKET_SIZE,
            TunTapMode::Tap => MAX_TAP_FRAME_SIZE,
        };
        if packet.len() > maximum_length {
            return Err(Error::MsgSize);
        }
        queue_packet(&mut packets.to_userspace, packet)
    }

    #[allow(dead_code)]
    pub(crate) fn dequeue_transmitted_packet(&self) -> VfsResult<Vec<u8>> {
        self.packets
            .lock()
            .to_kernel
            .pop_front()
            .ok_or(Error::Again)
    }

    pub(crate) fn is_readable(&self) -> bool {
        !self.packets.lock().to_userspace.is_empty()
    }

    pub(crate) fn is_writable(&self) -> bool {
        self.packets.lock().to_kernel.len() < MAX_QUEUED_PACKETS
    }
}

#[allow(dead_code)]
pub(crate) fn open_devices() -> VfsResult<Vec<Arc<TunDevice>>> {
    let mut registered = OPEN_TUN_DEVICES.lock();
    let mut devices = Vec::new();
    devices
        .try_reserve_exact(registered.len())
        .map_err(|_| Error::NoMem)?;
    registered.retain(|weak| {
        if let Some(device) = weak.upgrade() {
            devices.push(device);
            true
        } else {
            false
        }
    });
    Ok(devices)
}

fn queue_packet(queue: &mut VecDeque<Vec<u8>>, packet: &[u8]) -> VfsResult<()> {
    if packet.is_empty() {
        return Err(Error::Inval);
    }
    if packet.len() > MAX_TUN_PACKET_SIZE {
        return Err(Error::MsgSize);
    }
    if queue.len() >= MAX_QUEUED_PACKETS {
        return Err(Error::Again);
    }

    let mut owned_packet = Vec::new();
    owned_packet
        .try_reserve_exact(packet.len())
        .map_err(|_| Error::NoMem)?;
    owned_packet.extend_from_slice(packet);
    queue.try_reserve(1).map_err(|_| Error::NoMem)?;
    queue.push_back(owned_packet);
    Ok(())
}

pub(crate) struct TunDevicePlugin;

impl FilePlugin for TunDevicePlugin {
    fn open(
        &self,
        _mode: OpenMode,
    ) -> VfsFuture<'_, VfsResult<Option<Arc<dyn FilePlugin>>>> {
        Box::pin(async {
            let session: Arc<dyn FilePlugin> = Arc::new(TunDeviceFile {
                device: TunDevice::new()?,
            });
            Ok(Some(session))
        })
    }

    fn read_at<'a>(&'a self, _offset: u64, _buffer: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async { Err(Error::NoSys) })
    }
}

struct TunDeviceFile {
    device: Arc<TunDevice>,
}

impl FilePlugin for TunDeviceFile {
    fn read_at<'a>(&'a self, _offset: u64, buffer: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move { self.device.read_packet(buffer) })
    }

    fn write_at<'a>(&'a self, _offset: u64, buffer: &'a [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move { self.device.write_packet(buffer) })
    }
}