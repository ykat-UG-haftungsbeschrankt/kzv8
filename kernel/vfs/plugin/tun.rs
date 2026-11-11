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

use crate::kernel::error::Error;
use crate::kernel::scheduler::sync::IrqSpinLock;

use super::super::{FilePlugin, OpenMode, VfsFuture, VfsResult};

const MAX_TUN_PACKET_SIZE: usize = 65_535;
const MAX_QUEUED_PACKETS: usize = 64;

struct PacketQueues {
    to_userspace: VecDeque<Vec<u8>>,
    to_kernel: VecDeque<Vec<u8>>,
}

pub(crate) struct TunDevice {
    packets: IrqSpinLock<PacketQueues>,
}

static OPEN_TUN_DEVICES: IrqSpinLock<Vec<Weak<TunDevice>>> =
    IrqSpinLock::new(Vec::new());

impl TunDevice {
    fn new() -> VfsResult<Arc<Self>> {
        let device = Arc::new(Self {
            packets: IrqSpinLock::new(PacketQueues {
                to_userspace: VecDeque::new(),
                to_kernel: VecDeque::new(),
            }),
        });
        let mut devices = OPEN_TUN_DEVICES.lock();
        devices.retain(|registered| registered.strong_count() > 0);
        devices.try_reserve(1).map_err(|_| Error::NoMem)?;
        devices.push(Arc::downgrade(&device));
        Ok(device)
    }

    fn read_packet(&self, buffer: &mut [u8]) -> VfsResult<usize> {
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

    fn write_packet(&self, packet: &[u8]) -> VfsResult<usize> {
        let mut packets = self.packets.lock();
        queue_packet(&mut packets.to_kernel, packet)?;
        Ok(packet.len())
    }

    #[allow(dead_code)]
    pub(crate) fn inject_packet(&self, packet: &[u8]) -> VfsResult<()> {
        let mut packets = self.packets.lock();
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