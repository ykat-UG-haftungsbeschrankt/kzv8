use alloc::boxed::Box;

use crate::dma::{self, DmaBuffer};
use crate::virtio_queue::{Descriptor, VirtQueue};
use crate::virtio_transport::VirtioTransport;

const DEVICE_ID: u32 = 4;
const QUEUE_SIZE: usize = 8;
const MAX_CHUNK: usize = 256;
const COMPLETION_SPINS: usize = 10_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Error {
    Unavailable,
    QueueBusy,
    Timeout,
}

pub(crate) struct VirtioRng {
    transport: VirtioTransport,
    queue: Box<VirtQueue<QUEUE_SIZE>>,
    buffer: DmaBuffer,
}

impl VirtioRng {
    pub(crate) fn initialize() -> Option<Self> {
        let candidate = unsafe { VirtioTransport::find(DEVICE_ID)? };
        let transport = unsafe { VirtioTransport::initialize(candidate.base(), DEVICE_ID) }.ok()?;
        let mut queue = Box::new(VirtQueue::new());
        queue.configure(&transport, 0).ok()?;
        if !transport.activate() {
            return None;
        }
        Some(Self {
            transport,
            queue,
            buffer: dma::allocate(MAX_CHUNK, 16)?,
        })
    }

    pub(crate) fn fill(&mut self, output: &mut [u8]) -> Result<(), Error> {
        for chunk in output.chunks_mut(MAX_CHUNK) {
            let address = self.buffer.bus_address();
            let descriptor = Descriptor::new(address, chunk.len() as u32, true);
            let head = self.queue.submit(&[descriptor]).ok_or(Error::QueueBusy)?;
            dma::sync_for_device();
            self.transport.notify(0);
            self.queue
                .wait_used(head, COMPLETION_SPINS)
                .ok_or(Error::Timeout)?;
            dma::sync_for_cpu();
            chunk.copy_from_slice(&self.buffer.as_mut_slice()[..chunk.len()]);
        }
        Ok(())
    }
}
