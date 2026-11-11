/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::vm::device::virtio::mmio::{QueueResult, VirtioMmio};
use crate::kernel::vm::device::virtio::queue::GuestVirtQueue;
use crate::kernel::vm::device::GuestMemory;
use crate::kernel::vm::device::virtio::GuestInput;

pub(crate) const MMIO_BASE: u64 = 0x1000_0000;
const DEVICE_ID: u32 = 3;
const VENDOR_ID: u32 = 0x4b5a;
const MAX_TRANSFER_SIZE: usize = 512;

pub(crate) struct VirtioConsole {
    mmio: VirtioMmio,
}

impl VirtioConsole {
    pub(crate) const fn new() -> Self {
        Self {
            mmio: VirtioMmio::new(
                MMIO_BASE,
                DEVICE_ID,
                VENDOR_ID,
                [0, 1],
                [0; 16],
                [0; 16],
                2,
                256,
            ),
        }
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
        host_input: &mut GuestInput,
    ) -> Option<u64> {
        if !Self::contains(address) {
            return None;
        }
        let receive_result = self
            .mmio
            .queue_mut(0)
            .filter(|queue| queue.is_ready())
            .map(|queue| Self::receive_available(queue, memory, host_input));
        if matches!(receive_result, Some(QueueResult::Complete)) {
            self.mmio.signal_used_buffer();
        }
        self.mmio
            .access(memory, address, width, write, value, |queue_index, queue, memory| {
                Self::notify_queue(queue_index, queue, memory, host_input)
            })
    }

    fn notify_queue(
        queue_index: usize,
        queue: &mut GuestVirtQueue,
        memory: &mut GuestMemory<'_>,
        host_input: &mut GuestInput,
    ) -> QueueResult {
        match queue_index {
            0 => Self::receive_available(queue, memory, host_input),
            1 => Self::transmit_available(queue, memory),
            _ => QueueResult::Invalid,
        }
    }

    fn receive_available(
        queue: &mut GuestVirtQueue,
        memory: &mut GuestMemory<'_>,
        host_input: &mut GuestInput,
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
            let mut input = [0; MAX_TRANSFER_SIZE];
            let input_length = host_input.read_console(&mut input[..capacity.min(input.len())]);
            if input_length == 0 {
                break;
            }
            let Some(written) = chain.write_device_data(memory, &input[..input_length]) else {
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
            if capacity > MAX_TRANSFER_SIZE {
                return QueueResult::Invalid;
            }
            let mut output = [0; MAX_TRANSFER_SIZE];
            let Some(length) = chain.read_device_data(memory, &mut output) else {
                return QueueResult::Invalid;
            };
            if !crate::kernel::driver::virtio::console::Console::write(&output[..length]) {
                break;
            }
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