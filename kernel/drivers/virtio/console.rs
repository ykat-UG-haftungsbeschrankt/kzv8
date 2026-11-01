use core::sync::atomic::{AtomicUsize, Ordering};

const QUEUE_SIZE: usize = 8;
const VIRTIO_CONSOLE_DEVICE_ID: u32 = 3;
const COMPLETION_SPINS: usize = 10_000_000;

use crate::virtio_queue::{Descriptor, QueueCell};
use crate::virtio_transport::VirtioTransport;

static QUEUE: QueueCell<QUEUE_SIZE> = QueueCell::new();
static CONSOLE_BASE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn initialize() -> bool {
    let Some(candidate) = (unsafe { VirtioTransport::find(VIRTIO_CONSOLE_DEVICE_ID) }) else {
        return false;
    };
    let base = candidate.base();
    let Ok(transport) = (unsafe { VirtioTransport::initialize(base, VIRTIO_CONSOLE_DEVICE_ID) })
    else {
        return false;
    };
    if QUEUE.lock().configure(&transport, 1).is_err() || !transport.activate() {
        return false;
    }
    CONSOLE_BASE.store(base, Ordering::Release);
    true
}

pub(crate) fn write(message: &[u8]) -> bool {
    if message.len() > 512 {
        return false;
    }
    let base = CONSOLE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return false;
    }
    let mut output = [0u8; 512];
    output[..message.len()].copy_from_slice(message);
    crate::dma::sync_for_device();
    let descriptor = Descriptor::new(output.as_mut_ptr() as u64, message.len() as u32, false);
    let mut queue = QUEUE.lock();
    let Some(head) = queue.submit(&[descriptor]) else {
        return false;
    };
    VirtioTransport::from_base(base).notify(1);
    queue.wait_used(head, COMPLETION_SPINS).is_some()
}
