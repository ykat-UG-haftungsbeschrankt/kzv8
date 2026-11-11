/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::vm::device::virtio::mmio::{QueueResult, VirtioMmio};
use crate::kernel::vm::device::virtio::GuestInput;
use crate::kernel::vm::device::virtio::queue::GuestVirtQueue;
use crate::kernel::vm::device::GuestMemory;

pub(crate) const MMIO_BASE: u64 = 0x1000_6000;
const DEVICE_ID: u32 = 18;
const VENDOR_ID: u32 = 0x4b5a;
const CONFIG_SELECT_EVENT_BITS: u8 = 0x11;
const EVENT_SYN: u16 = 0;
const EVENT_KEY: u16 = 1;
const SYN_REPORT: u16 = 0;
const EVENT_BUFFER_SIZE: usize = 8;
const EVENT_RING_SIZE: usize = 64;
const CONFIG_BITMAP_SIZE: u8 = 8;
const CONFIG_DATA_OFFSET: usize = 9;

#[derive(Clone, Copy)]
struct InputEvent {
    event_type: u16,
    code: u16,
    value: u32,
}

impl InputEvent {
    const EMPTY: Self = Self {
        event_type: 0,
        code: 0,
        value: 0,
    };
}

pub(crate) struct VirtioInput {
    mmio: VirtioMmio,
    select: u8,
    subselect: u8,
    events: [InputEvent; EVENT_RING_SIZE],
    head: usize,
    length: usize,
}

impl VirtioInput {
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
            select: 0,
            subselect: 0,
            events: [InputEvent::EMPTY; EVENT_RING_SIZE],
            head: 0,
            length: 0,
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
        let offset = address - MMIO_BASE;
        if offset >= 0x100 {
            return self.access_config(offset, width, write, value);
        }
        let Self {
            mmio,
            events,
            head,
            length,
            ..
        } = self;
        let event_result = mmio
            .queue_mut(0)
            .filter(|queue| queue.is_ready())
            .map(|queue| Self::process_event_queue(events, head, length, queue, memory, host_input));
        if matches!(event_result, Some(QueueResult::Complete)) {
            mmio.signal_used_buffer();
        }
        mmio.access(memory, address, width, write, value, Self::notify_status_queue)
    }

    fn access_config(&mut self, offset: u64, width: u8, write: bool, value: u64) -> Option<u64> {
        if !matches!(width, 1 | 2 | 4) {
            return Some(0);
        }
        let config_offset = usize::try_from(offset.checked_sub(0x100)?).ok()?;
        let length = usize::from(width);
        if config_offset.checked_add(length)? > CONFIG_DATA_OFFSET + usize::from(CONFIG_BITMAP_SIZE) {
            return Some(0);
        }
        if write {
            for index in 0..length {
                let position = config_offset + index;
                let byte = (value >> (index * 8)) as u8;
                match position {
                    0 => self.select = byte,
                    1 => self.subselect = byte,
                    _ => {}
                }
            }
            return Some(0);
        }
        let mut result = 0u64;
        for index in 0..length {
            let position = config_offset + index;
            let byte = match position {
                0 => self.select,
                1 => self.subselect,
                2 => self.config_size(),
                8 => self.config_size(),
                _ => {
                    if position < CONFIG_DATA_OFFSET {
                        continue;
                    }
                    let data_offset = position - CONFIG_DATA_OFFSET;
                    let word = self.config_word(data_offset / 4);
                    (word >> ((data_offset % 4) * 8)) as u8
                }
            };
            result |= u64::from(byte) << (index * 8);
        }
        Some(result)
    }

    fn config_size(&self) -> u8 {
        if self.select != CONFIG_SELECT_EVENT_BITS {
            return 0;
        }
        match self.subselect {
            0 => 1,
            1 => CONFIG_BITMAP_SIZE,
            _ => 0,
        }
    }

    fn config_word(&self, word: usize) -> u32 {
        if self.select != CONFIG_SELECT_EVENT_BITS {
            return 0;
        }
        match self.subselect {
            0 if word == 0 => 1,
            1 => {
                let mut bitmap = 0u32;
                for bit in 0..32 {
                    let code = word.saturating_mul(32).saturating_add(bit);
                    if Self::supported_key(code as u16) {
                        bitmap |= 1 << bit;
                    }
                }
                bitmap
            }
            _ => 0,
        }
    }

    fn supported_key(code: u16) -> bool {
        matches!(code, 2..=11 | 14 | 16..=25 | 28 | 30..=38 | 44..=50 | 57)
    }

    fn notify_status_queue(
        queue_index: usize,
        queue: &mut GuestVirtQueue,
        memory: &mut GuestMemory<'_>,
    ) -> QueueResult {
        if queue_index == 0 {
            return QueueResult::Pending;
        }
        if queue_index != 1 {
            return QueueResult::Invalid;
        }
        let mut completed = false;
        loop {
            let chain = match queue.available_chain(memory) {
                Ok(Some(chain)) => chain,
                Ok(None) => break,
                Err(()) => return QueueResult::Invalid,
            };
            let Some(length) = chain.readable_capacity() else {
                return QueueResult::Invalid;
            };
            if length > EVENT_BUFFER_SIZE {
                return QueueResult::Invalid;
            }
            let mut discard = [0; EVENT_BUFFER_SIZE];
            if chain.read_device_data(memory, &mut discard).is_none()
                || !queue.complete(memory, &chain, 0)
            {
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

    fn process_event_queue(
        events: &mut [InputEvent; EVENT_RING_SIZE],
        head: &mut usize,
        length: &mut usize,
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
            if chain.writable_capacity().is_none_or(|capacity| capacity < EVENT_BUFFER_SIZE) {
                return QueueResult::Invalid;
            }
            if *length == 0 {
                Self::read_host_character(events, head, length, host_input);
            }
            let Some(event) = Self::pop_event(events, head, length) else {
                break;
            };
            let bytes = Self::encode_event(event);
            let Some(written) = chain.write_device_data(memory, &bytes) else {
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

    fn read_host_character(
        events: &mut [InputEvent; EVENT_RING_SIZE],
        head: &mut usize,
        length: &mut usize,
        host_input: &mut GuestInput,
    ) {
        let Some(byte) = host_input.read_keyboard_byte() else {
            return;
        };
        let Some(code) = Self::key_code(byte) else {
            return;
        };
        Self::push_event(events, head, length, InputEvent {
            event_type: EVENT_KEY,
            code,
            value: 1,
        });
        Self::push_event(events, head, length, InputEvent {
            event_type: EVENT_SYN,
            code: SYN_REPORT,
            value: 0,
        });
        Self::push_event(events, head, length, InputEvent {
            event_type: EVENT_KEY,
            code,
            value: 0,
        });
        Self::push_event(events, head, length, InputEvent {
            event_type: EVENT_SYN,
            code: SYN_REPORT,
            value: 0,
        });
    }

    fn key_code(byte: u8) -> Option<u16> {
        let byte = byte.to_ascii_lowercase();
        Some(match byte {
            b'1'..=b'9' => u16::from(byte - b'1') + 2,
            b'0' => 11,
            b'a'..=b'z' => match byte {
                b'a' => 30, b'b' => 48, b'c' => 46, b'd' => 32, b'e' => 18, b'f' => 33,
                b'g' => 34, b'h' => 35, b'i' => 23, b'j' => 36, b'k' => 37, b'l' => 38,
                b'm' => 50, b'n' => 49, b'o' => 24, b'p' => 25, b'q' => 16, b'r' => 19,
                b's' => 31, b't' => 20, b'u' => 22, b'v' => 47, b'w' => 17, b'x' => 45,
                b'y' => 21, b'z' => 44, _ => return None,
            },
            b'\n' | b'\r' => 28,
            b' ' => 57,
            0x08 | 0x7f => 14,
            _ => return None,
        })
    }

    fn push_event(
        events: &mut [InputEvent; EVENT_RING_SIZE],
        head: &usize,
        length: &mut usize,
        event: InputEvent,
    ) {
        if *length == EVENT_RING_SIZE {
            return;
        }
        let tail = (*head + *length) % EVENT_RING_SIZE;
        events[tail] = event;
        *length += 1;
    }

    fn pop_event(
        events: &[InputEvent; EVENT_RING_SIZE],
        head: &mut usize,
        length: &mut usize,
    ) -> Option<InputEvent> {
        if *length == 0 {
            return None;
        }
        let event = events[*head];
        *head = (*head + 1) % EVENT_RING_SIZE;
        *length -= 1;
        Some(event)
    }

    fn encode_event(event: InputEvent) -> [u8; EVENT_BUFFER_SIZE] {
        let mut bytes = [0; EVENT_BUFFER_SIZE];
        bytes[..2].copy_from_slice(&event.event_type.to_le_bytes());
        bytes[2..4].copy_from_slice(&event.code.to_le_bytes());
        bytes[4..8].copy_from_slice(&event.value.to_le_bytes());
        bytes
    }
}
