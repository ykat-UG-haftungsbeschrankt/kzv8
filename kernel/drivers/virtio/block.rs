use alloc::boxed::Box;

use crate::virtio_queue::{Descriptor, VirtQueue};
use crate::virtio_transport::VirtioTransport;

const DEVICE_ID: u32 = 2;
const QUEUE_SIZE: usize = 8;
const SECTOR_SIZE: usize = 512;
const COMPLETION_SPINS: usize = 10_000_000;

#[repr(C)]
struct RequestHeader {
    request_type: u32,
    reserved: u32,
    sector: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Error {
    Unavailable,
    InvalidBuffer,
    QueueBusy,
    Timeout,
    DeviceStatus(u8),
}

pub(crate) struct BlockDevice {
    transport: VirtioTransport,
    queue: Box<VirtQueue<QUEUE_SIZE>>,
    capacity_sectors: u64,
}

impl BlockDevice {
    pub(crate) fn initialize() -> Option<Self> {
        let candidate = unsafe { VirtioTransport::find(DEVICE_ID)? };
        let transport = unsafe { VirtioTransport::initialize(candidate.base(), DEVICE_ID) }.ok()?;
        let mut queue = Box::new(VirtQueue::new());
        queue.configure(&transport, 0).ok()?;
        if !transport.activate() {
            return None;
        }
        let capacity_sectors = u64::from(transport.read_device_config(0))
            | (u64::from(transport.read_device_config(4)) << 32);
        Some(Self {
            transport,
            queue,
            capacity_sectors,
        })
    }

    pub(crate) const fn capacity_sectors(&self) -> u64 {
        self.capacity_sectors
    }

    pub(crate) fn read_sectors(&mut self, sector: u64, output: &mut [u8]) -> Result<(), Error> {
        if output.is_empty() || output.len() % SECTOR_SIZE != 0 {
            return Err(Error::InvalidBuffer);
        }
        self.transfer(0, sector, output.as_mut_ptr(), output.len(), true)
    }

    pub(crate) fn write_sectors(&mut self, sector: u64, input: &[u8]) -> Result<(), Error> {
        if input.is_empty() || input.len() % SECTOR_SIZE != 0 {
            return Err(Error::InvalidBuffer);
        }
        self.transfer(1, sector, input.as_ptr() as *mut u8, input.len(), false)
    }

    fn transfer(
        &mut self,
        request_type: u32,
        sector: u64,
        data: *mut u8,
        length: usize,
        device_writes: bool,
    ) -> Result<(), Error> {
        if length > u32::MAX as usize {
            return Err(Error::InvalidBuffer);
        }
        if sector
            .checked_add((length / SECTOR_SIZE) as u64)
            .is_none_or(|end| end > self.capacity_sectors)
        {
            return Err(Error::InvalidBuffer);
        }
        let header = RequestHeader {
            request_type,
            reserved: 0,
            sector,
        };
        let mut status = 0xffu8;
        let descriptors = [
            Descriptor::new(
                &header as *const _ as u64,
                core::mem::size_of::<RequestHeader>() as u32,
                false,
            ),
            Descriptor::new(data as u64, length as u32, device_writes),
            Descriptor::new(&mut status as *mut u8 as u64, 1, true),
        ];
        let head = self.queue.submit(&descriptors).ok_or(Error::QueueBusy)?;
        crate::dma::sync_for_device();
        self.transport.notify(0);
        self.queue
            .wait_used(head, COMPLETION_SPINS)
            .ok_or(Error::Timeout)?;
        if status == 0 {
            Ok(())
        } else {
            Err(Error::DeviceStatus(status))
        }
    }
}
