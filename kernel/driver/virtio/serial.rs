/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use of this file is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::sync::atomic::{AtomicUsize, Ordering};

use crate::kernel::driver::virtio::queue::{Descriptor, QueueCell};
use crate::kernel::driver::virtio::transport::VirtioTransport;
use crate::kernel::memory::dma::{DmaArena, DmaBuffer};
use crate::kernel::scheduler::sync::IrqSpinLock;

struct InputQueue {
    bytes: [u8; VirtioSerial::INPUT_CAPACITY],
    head: usize,
    length: usize,
    active_head: Option<u16>,
}

impl InputQueue {
    const EMPTY: Self = Self {
        bytes: [0; VirtioSerial::INPUT_CAPACITY],
        head: 0,
        length: 0,
        active_head: None,
    };
}

struct OutputQueue {
    bytes: [u8; VirtioSerial::OUTPUT_CAPACITY],
    head: usize,
    length: usize,
    active_length: usize,
    active_head: Option<u16>,
}

impl OutputQueue {
    const EMPTY: Self = Self {
        bytes: [0; VirtioSerial::OUTPUT_CAPACITY],
        head: 0,
        length: 0,
        active_length: 0,
        active_head: None,
    };
}

pub(crate) struct VirtioSerial {
    receive_queue: QueueCell<{ Self::QUEUE_SIZE }>,
    transmit_queue: QueueCell<{ Self::QUEUE_SIZE }>,
    base: AtomicUsize,
    slot: AtomicUsize,
    input: IrqSpinLock<InputQueue>,
    output: IrqSpinLock<OutputQueue>,
    receive_buffer: IrqSpinLock<Option<DmaBuffer>>,
}

impl VirtioSerial {
    const DEVICE_ID: u32 = 3;
    const QUEUE_SIZE: usize = 8;
    const INPUT_CAPACITY: usize = 16 * 1024;
    const OUTPUT_CAPACITY: usize = 16 * 1024;
    const MAX_CHUNK_SIZE: usize = 512;

    const fn new() -> Self {
        Self {
            receive_queue: QueueCell::new(),
            transmit_queue: QueueCell::new(),
            base: AtomicUsize::new(0),
            slot: AtomicUsize::new(usize::MAX),
            input: IrqSpinLock::new(InputQueue::EMPTY),
            output: IrqSpinLock::new(OutputQueue::EMPTY),
            receive_buffer: IrqSpinLock::new(None),
        }
    }

    fn get_instance() -> &'static Self {
        static SERIAL: VirtioSerial = VirtioSerial::new();
        &SERIAL
    }

    pub(crate) fn initialize() -> bool {
        let serial = Self::get_instance();
        if serial.base.load(Ordering::Acquire) != 0 {
            return true;
        }

        let Some(candidate) = VirtioTransport::find(Self::DEVICE_ID) else {
            return false;
        };
        let Some(receive_buffer) = DmaArena::allocate(Self::MAX_CHUNK_SIZE, 16) else {
            return false;
        };
        let Ok(transport) = candidate.initialize(Self::DEVICE_ID) else {
            return false;
        };
        if serial
            .receive_queue
            .lock()
            .configure(&transport, 0)
            .is_err()
            || serial
                .transmit_queue
                .lock()
                .configure(&transport, 1)
                .is_err()
            || !transport.activate()
        {
            return false;
        }

        *serial.receive_buffer.lock() = Some(receive_buffer);
        let base = transport.base();
        serial.base.store(base, Ordering::Release);
        serial.slot.store(transport.slot(), Ordering::Release);

        let mut input = serial.input.lock();
        let notify = serial.submit_receive(&mut input);
        drop(input);
        if !notify {
            return false;
        }
        transport.notify(0);
        true
    }

    pub(crate) fn write(message: &[u8]) -> bool {
        let serial = Self::get_instance();
        if message.len() > Self::MAX_CHUNK_SIZE {
            return false;
        }
        if message.is_empty() {
            return true;
        }
        let base = serial.base.load(Ordering::Acquire);
        if base == 0 {
            return false;
        }

        let mut output = serial.output.lock();
        if message.len() > Self::OUTPUT_CAPACITY - output.length {
            return false;
        }
        let tail = (output.head + output.length) % Self::OUTPUT_CAPACITY;
        let first_length = message.len().min(Self::OUTPUT_CAPACITY - tail);
        output.bytes[tail..tail + first_length].copy_from_slice(&message[..first_length]);
        output.bytes[..message.len() - first_length].copy_from_slice(&message[first_length..]);
        output.length += message.len();
        let notify = serial.submit_transmit(&mut output);
        drop(output);
        if notify {
            if let Some(transport) = VirtioTransport::from_base(base) {
                transport.notify(1);
            }
        }
        true
    }

    #[allow(dead_code)]
    pub(crate) fn read(output: &mut [u8]) -> usize {
        let serial = Self::get_instance();
        let base = serial.base.load(Ordering::Acquire);
        if base == 0 || output.is_empty() {
            return 0;
        }

        let mut input = serial.input.lock();
        let length = output.len().min(input.length);
        let first_length = length.min(Self::INPUT_CAPACITY - input.head);
        output[..first_length].copy_from_slice(&input.bytes[input.head..input.head + first_length]);
        output[first_length..length].copy_from_slice(&input.bytes[..length - first_length]);
        input.head = (input.head + length) % Self::INPUT_CAPACITY;
        input.length -= length;
        let notify = serial.submit_receive(&mut input);
        drop(input);
        if notify {
            if let Some(transport) = VirtioTransport::from_base(base) {
                transport.notify(0);
            }
        }
        length
    }

    pub(crate) fn handle_interrupt(pending: u64) {
        let serial = Self::get_instance();
        let slot = serial.slot.load(Ordering::Acquire);
        if slot >= 64 || pending & (1u64 << slot) == 0 {
            return;
        }
        let base = serial.base.load(Ordering::Acquire);
        if base == 0 {
            return;
        }

        let mut input = serial.input.lock();
        if let (Some(expected), Some(used)) =
            (input.active_head, serial.receive_queue.lock().poll_used())
        {
            input.active_head = None;
            if used.id == u32::from(expected) {
                let length = (used.length as usize).min(Self::MAX_CHUNK_SIZE);
                if length != 0 {
                    let mut receive_buffer = serial.receive_buffer.lock();
                    if let Some(buffer) = receive_buffer.as_mut() {
                        Self::append_input(&mut input, &buffer.as_mut_slice()[..length]);
                    }
                }
            }
        }
        let receive_notify = serial.submit_receive(&mut input);
        drop(input);

        let mut output = serial.output.lock();
        if let Some(expected) = output.active_head {
            if let Some(used) = serial.transmit_queue.lock().poll_used() {
                if used.id == u32::from(expected) {
                    output.head = (output.head + output.active_length) % Self::OUTPUT_CAPACITY;
                    output.length -= output.active_length;
                }
                output.active_length = 0;
                output.active_head = None;
            }
        }
        let transmit_notify = serial.submit_transmit(&mut output);
        drop(output);

        if receive_notify || transmit_notify {
            if let Some(transport) = VirtioTransport::from_base(base) {
                if receive_notify {
                    transport.notify(0);
                }
                if transmit_notify {
                    transport.notify(1);
                }
            }
        }
    }

    fn append_input(input: &mut InputQueue, bytes: &[u8]) {
        let length = bytes.len().min(Self::INPUT_CAPACITY - input.length);
        let tail = (input.head + input.length) % Self::INPUT_CAPACITY;
        let first_length = length.min(Self::INPUT_CAPACITY - tail);
        input.bytes[tail..tail + first_length].copy_from_slice(&bytes[..first_length]);
        input.bytes[..length - first_length].copy_from_slice(&bytes[first_length..length]);
        input.length += length;
    }

    fn submit_receive(&self, input: &mut InputQueue) -> bool {
        if input.active_head.is_some() || input.length == Self::INPUT_CAPACITY {
            return false;
        }
        let length = (Self::INPUT_CAPACITY - input.length).min(Self::MAX_CHUNK_SIZE);
        let receive_buffer = self.receive_buffer.lock();
        let Some(buffer) = receive_buffer.as_ref() else {
            return false;
        };
        let descriptor = Descriptor::new(buffer.bus_address(), length as u32, true);
        let Some(head) = self.receive_queue.lock().submit(&[descriptor]) else {
            return false;
        };
        input.active_head = Some(head);
        DmaArena::sync_for_device();
        true
    }

    fn submit_transmit(&self, output: &mut OutputQueue) -> bool {
        if output.active_head.is_some() || output.length == 0 {
            return false;
        }
        let length = output
            .length
            .min(Self::MAX_CHUNK_SIZE)
            .min(Self::OUTPUT_CAPACITY - output.head);
        let descriptor = Descriptor::new(
            output.bytes[output.head..output.head + length].as_ptr() as u64,
            length as u32,
            false,
        );
        let Some(head) = self.transmit_queue.lock().submit(&[descriptor]) else {
            return false;
        };
        output.active_head = Some(head);
        output.active_length = length;
        DmaArena::sync_for_device();
        true
    }
}
