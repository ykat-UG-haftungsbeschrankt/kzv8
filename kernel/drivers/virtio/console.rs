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

use crate::sync::IrqSpinLock;
use crate::virtio_queue::{Descriptor, QueueCell};
use crate::virtio_transport::VirtioTransport;

static QUEUE: QueueCell<QUEUE_SIZE> = QueueCell::new();
static CONSOLE_BASE: AtomicUsize = AtomicUsize::new(0);
static CONSOLE_SLOT: AtomicUsize = AtomicUsize::new(usize::MAX);
static OUTPUT: IrqSpinLock<OutputQueue> = IrqSpinLock::new(OutputQueue::EMPTY);

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
    CONSOLE_SLOT.store(transport.slot(), Ordering::Release);
    true
}

pub(crate) fn write(message: &[u8]) -> bool {
    if message.len() > MAX_CHUNK_SIZE {
        return false;
    }
    if message.is_empty() {
        return true;
    }
    let base = CONSOLE_BASE.load(Ordering::Acquire);
    if base == 0 {
        return false;
    }
    let mut output = OUTPUT.lock();
    if message.len() > OUTPUT_CAPACITY - output.length {
        return false;
    }
    let tail = (output.head + output.length) % OUTPUT_CAPACITY;
    let first_length = message.len().min(OUTPUT_CAPACITY - tail);
    output.bytes[tail..tail + first_length].copy_from_slice(&message[..first_length]);
    output.bytes[..message.len() - first_length].copy_from_slice(&message[first_length..]);
    output.length += message.len();
    let notify = submit_next(&mut output);
    drop(output);
    if notify {
        VirtioTransport::from_base(base).notify(1);
    }
    true
}

pub(crate) fn handle_interrupt(pending: u64) {
    let slot = CONSOLE_SLOT.load(Ordering::Acquire);
    if slot >= 64 || pending & (1u64 << slot) == 0 {
        return;
    }
    let base = CONSOLE_BASE.load(Ordering::Acquire);
    let mut output = OUTPUT.lock();
    let Some(expected) = output.active_head else {
        return;
    };
    let Some(used) = QUEUE.lock().poll_used() else {
        return;
    };
    if used.id != u32::from(expected) {
        return;
    }
    output.head = (output.head + output.active_length) % OUTPUT_CAPACITY;
    output.length -= output.active_length;
    output.active_length = 0;
    output.active_head = None;
    let notify = submit_next(&mut output);
    drop(output);
    if notify {
        VirtioTransport::from_base(base).notify(1);
    }
}

fn submit_next(output: &mut OutputQueue) -> bool {
    if output.active_head.is_some() || output.length == 0 {
        return false;
    }
    let length = output
        .length
        .min(MAX_CHUNK_SIZE)
        .min(OUTPUT_CAPACITY - output.head);
    let descriptor = Descriptor::new(
        unsafe { output.bytes.as_ptr().add(output.head) } as u64,
        length as u32,
        false,
    );
    let Some(head) = QUEUE.lock().submit(&[descriptor]) else {
        return false;
    };
    output.active_head = Some(head);
    output.active_length = length;
    crate::dma::sync_for_device();
    true
}
