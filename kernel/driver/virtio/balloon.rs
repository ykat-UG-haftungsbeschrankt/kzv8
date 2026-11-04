use alloc::vec::Vec;

use crate::kernel::driver::virtio::queue::{Descriptor, DmaQueue};
use crate::kernel::driver::virtio::transport::VirtioTransport;
use crate::kernel::error::Error;
use crate::kernel::memory::dma::{DmaArena, DmaBuffer};
use crate::kernel::memory::physical::{Frame, PhysicalAllocator};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Inflate,
    Deflate,
}

impl Operation {
    const fn queue_index(self) -> u32 {
        match self {
            Self::Inflate => 0,
            Self::Deflate => 1,
        }
    }
}

struct PendingOperation {
    operation: Operation,
    head: u16,
    frames: Vec<Frame>,
}

pub(crate) struct VirtioBalloon {
    transport: VirtioTransport,
    inflate_queue: DmaQueue<{ Self::QUEUE_SIZE }>,
    deflate_queue: DmaQueue<{ Self::QUEUE_SIZE }>,
    page_list: DmaBuffer,
    ballooned_pages: Vec<Frame>,
    pending: Option<PendingOperation>,
}

impl VirtioBalloon {
    const DEVICE_ID: u32 = 5;
    const QUEUE_SIZE: usize = 16;
    const MAX_BATCH_PAGES: usize = Self::QUEUE_SIZE;
    const PAGE_SIZE: usize = 4096;
    const TARGET_CONFIG_OFFSET: usize = 0;
    const ACTUAL_CONFIG_OFFSET: usize = 4;

    pub(crate) fn initialize() -> Option<Self> {
        let candidate = VirtioTransport::find(Self::DEVICE_ID)?;
        let transport = candidate.initialize(Self::DEVICE_ID).ok()?;
        let mut inflate_queue = DmaQueue::new()?;
        let mut deflate_queue = DmaQueue::new()?;
        let page_list = DmaArena::allocate(Self::MAX_BATCH_PAGES * size_of::<u32>(), 4)?;
        inflate_queue.configure(&transport, 0).ok()?;
        deflate_queue.configure(&transport, 1).ok()?;
        if !transport.activate() {
            return None;
        }
        transport.write_device_config(Self::ACTUAL_CONFIG_OFFSET, 0)?;
        Some(Self {
            transport,
            inflate_queue,
            deflate_queue,
            page_list,
            ballooned_pages: Vec::new(),
            pending: None,
        })
    }

    pub(crate) fn poll(&mut self) -> Result<bool, Error> {
        let mut progressed = false;
        if self.pending.is_some() {
            if !self.complete_pending()? {
                return Ok(false);
            }
            progressed = true;
        }

        let target = self
            .transport
            .read_device_config(Self::TARGET_CONFIG_OFFSET)
            .ok_or(Error::Io)?;
        let actual = self
            .transport
            .read_device_config(Self::ACTUAL_CONFIG_OFFSET)
            .ok_or(Error::Io)?;
        let managed = u32::try_from(self.ballooned_pages.len()).map_err(|_| Error::Overflow)?;
        if actual != managed {
            return Err(Error::Proto);
        }

        if target > actual {
            let count = (target - actual).min(Self::MAX_BATCH_PAGES as u32) as usize;
            return self.inflate(count).map(|submitted| submitted || progressed);
        }
        if target < actual {
            let count = (actual - target).min(Self::MAX_BATCH_PAGES as u32) as usize;
            return self.deflate(count).map(|submitted| submitted || progressed);
        }
        Ok(progressed)
    }

    fn inflate(&mut self, count: usize) -> Result<bool, Error> {
        self.ballooned_pages
            .try_reserve(count)
            .map_err(|_| Error::NoMem)?;
        let mut frames = Vec::new();
        frames.try_reserve_exact(count).map_err(|_| Error::NoMem)?;
        for _ in 0..count {
            let Some(frame) = PhysicalAllocator::allocate() else {
                break;
            };
            if u32::try_from(frame.address() / Self::PAGE_SIZE).is_err() {
                let _released = PhysicalAllocator::deallocate(frame);
                break;
            }
            frames.push(frame);
        }
        if frames.is_empty() {
            return Ok(false);
        }
        self.submit(Operation::Inflate, frames)
    }

    fn deflate(&mut self, count: usize) -> Result<bool, Error> {
        let mut frames = Vec::new();
        frames.try_reserve_exact(count).map_err(|_| Error::NoMem)?;
        let start = self
            .ballooned_pages
            .len()
            .checked_sub(count)
            .ok_or(Error::Proto)?;
        frames.extend_from_slice(&self.ballooned_pages[start..]);
        self.submit(Operation::Deflate, frames)
    }

    fn submit(&mut self, operation: Operation, frames: Vec<Frame>) -> Result<bool, Error> {
        let byte_length = frames
            .len()
            .checked_mul(size_of::<u32>())
            .ok_or(Error::Overflow)?;
        {
            let buffer = self.page_list.as_mut_slice();
            for (index, frame) in frames.iter().enumerate() {
                let page_number = u32::try_from(frame.address() / Self::PAGE_SIZE)
                    .map_err(|_| Error::Overflow)?;
                let start = index * size_of::<u32>();
                buffer[start..start + size_of::<u32>()]
                    .copy_from_slice(&page_number.to_le_bytes());
            }
        }

        let descriptor = [Descriptor::new(
            self.page_list.bus_address(),
            byte_length as u32,
            false,
        )];
        let head = match operation {
            Operation::Inflate => self.inflate_queue.submit(&descriptor),
            Operation::Deflate => self.deflate_queue.submit(&descriptor),
        };
        let Some(head) = head else {
            if operation == Operation::Inflate {
                Self::release_frames(&frames);
            }
            return Err(Error::Busy);
        };
        DmaArena::sync_for_device();
        self.transport.notify(operation.queue_index());
        self.pending = Some(PendingOperation {
            operation,
            head,
            frames,
        });
        Ok(true)
    }

    fn complete_pending(&mut self) -> Result<bool, Error> {
        let Some(pending) = self.pending.as_ref() else {
            return Ok(true);
        };
        let operation = pending.operation;
        let head = pending.head;
        let used = match operation {
            Operation::Inflate => self.inflate_queue.poll_used(),
            Operation::Deflate => self.deflate_queue.poll_used(),
        };
        let Some(used) = used else {
            return Ok(false);
        };
        let mut pending = self.pending.take().ok_or(Error::Proto)?;
        if used.id != u32::from(head) {
            if operation == Operation::Inflate {
                Self::release_frames(&pending.frames);
            }
            return Err(Error::Proto);
        }

        match operation {
            Operation::Inflate => self.ballooned_pages.append(&mut pending.frames),
            Operation::Deflate => {
                let start = self
                    .ballooned_pages
                    .len()
                    .checked_sub(pending.frames.len())
                    .ok_or(Error::Proto)?;
                if self.ballooned_pages.get(start..) != Some(pending.frames.as_slice()) {
                    return Err(Error::Proto);
                }
                self.ballooned_pages.truncate(start);
                for frame in pending.frames.into_iter().rev() {
                    if !PhysicalAllocator::deallocate(frame) {
                        return Err(Error::Proto);
                    }
                }
            }
        }
        let actual = u32::try_from(self.ballooned_pages.len()).map_err(|_| Error::Overflow)?;
        self.transport
            .write_device_config(Self::ACTUAL_CONFIG_OFFSET, actual)
            .ok_or(Error::Io)?;
        Ok(true)
    }

    fn release_frames(frames: &[Frame]) {
        for frame in frames {
            let _released = PhysicalAllocator::deallocate(*frame);
        }
    }
}