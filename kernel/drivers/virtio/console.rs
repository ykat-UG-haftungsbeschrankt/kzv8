/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::sync::atomic::{AtomicUsize, Ordering};

const QUEUE_SIZE: usize = 8;
const OUTPUT_CAPACITY: usize = 16 * 1024;
const MAX_CHUNK_SIZE: usize = 512;
const VIRTIO_CONSOLE_DEVICE_ID: u32 = 3;

use crate::kernel::drivers::virtio::queue::{Descriptor, QueueCell};
use crate::kernel::drivers::virtio::transport::VirtioTransport;
use crate::kernel::scheduler::sync::IrqSpinLock;

struct OutputQueue {
    bytes: [u8; OUTPUT_CAPACITY],
    head: usize,
    length: usize,
    active_length: usize,
    active_head: Option<u16>,
}

impl OutputQueue {
    const EMPTY: Self = Self {
        bytes: [0; OUTPUT_CAPACITY],
        head: 0,
        length: 0,
        active_length: 0,
        active_head: None,
    };
}

pub(crate) struct Console {
    queue: QueueCell<QUEUE_SIZE>,
    base: AtomicUsize,
    slot: AtomicUsize,
    output: IrqSpinLock<OutputQueue>,
}

impl Console {
    const fn new() -> Self {
        Self {
            queue: QueueCell::new(),
            base: AtomicUsize::new(0),
            slot: AtomicUsize::new(usize::MAX),
            output: IrqSpinLock::new(OutputQueue::EMPTY),
        }
    }

    fn global() -> &'static Self {
        static CONSOLE: Console = Console::new();
        &CONSOLE
    }

    pub(crate) fn initialize() -> bool {
        let console = Self::global();
        let Some(candidate) = VirtioTransport::find(VIRTIO_CONSOLE_DEVICE_ID) else {
            return false;
        };
        let Ok(transport) = candidate.initialize(VIRTIO_CONSOLE_DEVICE_ID) else {
            return false;
        };
        let base = transport.base();
        if console.queue.lock().configure(&transport, 1).is_err() || !transport.activate() {
            return false;
        }
        console.base.store(base, Ordering::Release);
        console.slot.store(transport.slot(), Ordering::Release);
        true
    }

    pub(crate) fn write(message: &[u8]) -> bool {
        let console = Self::global();
        if message.len() > MAX_CHUNK_SIZE {
            return false;
        }
        if message.is_empty() {
            return true;
        }
        let base = console.base.load(Ordering::Acquire);
        if base == 0 {
            return false;
        }
        let mut output = console.output.lock();
        if message.len() > OUTPUT_CAPACITY - output.length {
            return false;
        }
        let tail = (output.head + output.length) % OUTPUT_CAPACITY;
        let first_length = message.len().min(OUTPUT_CAPACITY - tail);
        output.bytes[tail..tail + first_length].copy_from_slice(&message[..first_length]);
        output.bytes[..message.len() - first_length].copy_from_slice(&message[first_length..]);
        output.length += message.len();
        let notify = console.submit_next(&mut output);
        drop(output);
        if notify {
            if let Some(transport) = VirtioTransport::from_base(base) {
                transport.notify(1);
            }
        }
        true
    }

    pub(crate) fn handle_interrupt(pending: u64) {
        let console = Self::global();
        let slot = console.slot.load(Ordering::Acquire);
        if slot >= 64 || pending & (1u64 << slot) == 0 {
            return;
        }
        let base = console.base.load(Ordering::Acquire);
        let mut output = console.output.lock();
        let Some(expected) = output.active_head else {
            return;
        };
        let Some(used) = console.queue.lock().poll_used() else {
            return;
        };
        if used.id != u32::from(expected) {
            return;
        }
        output.head = (output.head + output.active_length) % OUTPUT_CAPACITY;
        output.length -= output.active_length;
        output.active_length = 0;
        output.active_head = None;
        let notify = console.submit_next(&mut output);
        drop(output);
        if notify {
            if let Some(transport) = VirtioTransport::from_base(base) {
                transport.notify(1);
            }
        }
    }

    fn submit_next(&self, output: &mut OutputQueue) -> bool {
        if output.active_head.is_some() || output.length == 0 {
            return false;
        }
        let length = output
            .length
            .min(MAX_CHUNK_SIZE)
            .min(OUTPUT_CAPACITY - output.head);
        let descriptor = Descriptor::new(
            output.bytes[output.head..output.head + length].as_ptr() as u64,
            length as u32,
            false,
        );
        let Some(head) = self.queue.lock().submit(&[descriptor]) else {
            return false;
        };
        output.active_head = Some(head);
        output.active_length = length;
        crate::kernel::memory::dma::sync_for_device();
        true
    }
}
