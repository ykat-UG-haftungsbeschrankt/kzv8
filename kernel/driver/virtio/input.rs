/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use of this file is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::kernel::driver::virtio::queue::{Descriptor, QueueCell};
use crate::kernel::driver::virtio::transport::VirtioTransport;
use crate::kernel::memory::dma::{DmaArena, DmaBuffer};
use crate::kernel::scheduler::sync::IrqSpinLock;

pub(crate) const EV_SYN: u16 = 0;
pub(crate) const EV_KEY: u16 = 1;
pub(crate) const EV_REL: u16 = 2;
pub(crate) const EV_ABS: u16 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InputEvent {
    pub(crate) event_type: u16,
    pub(crate) code: u16,
    pub(crate) value: u32,
}

impl InputEvent {
    const EMPTY: Self = Self {
        event_type: 0,
        code: 0,
        value: 0,
    };
}

struct EventRing {
    events: [InputEvent; VirtioInput::EVENT_CAPACITY],
    head: usize,
    length: usize,
    active_head: Option<u16>,
}

impl EventRing {
    const EMPTY: Self = Self {
        events: [InputEvent::EMPTY; VirtioInput::EVENT_CAPACITY],
        head: 0,
        length: 0,
        active_head: None,
    };

    fn push(&mut self, event: InputEvent) -> bool {
        if self.length == VirtioInput::EVENT_CAPACITY {
            return false;
        }
        let tail = (self.head + self.length) % VirtioInput::EVENT_CAPACITY;
        self.events[tail] = event;
        self.length += 1;
        true
    }

    fn pop(&mut self) -> Option<InputEvent> {
        if self.length == 0 {
            return None;
        }
        let event = self.events[self.head];
        self.head = (self.head + 1) % VirtioInput::EVENT_CAPACITY;
        self.length -= 1;
        Some(event)
    }
}

pub(crate) struct VirtioInput {
    event_queue: QueueCell<{ Self::QUEUE_SIZE }>,
    status_queue: QueueCell<{ Self::QUEUE_SIZE }>,
    base: AtomicUsize,
    slot: AtomicUsize,
    supported_event_types: AtomicU32,
    events: IrqSpinLock<EventRing>,
    event_buffer: IrqSpinLock<Option<DmaBuffer>>,
}

impl VirtioInput {
    const DEVICE_ID: u32 = 18;
    const QUEUE_SIZE: usize = 8;
    const EVENT_CAPACITY: usize = 128;
    const EVENT_SIZE: usize = 8;
    const MAX_CONFIG_BITMAP_SIZE: u8 = 128;
    const CONFIG_SELECT_OFFSET: usize = 0;
    const CONFIG_SUBSELECT_OFFSET: usize = 1;
    const CONFIG_SIZE_OFFSET: usize = 2;
    const CONFIG_SELECT_EVENT_BITS: u8 = 0x11;

    const fn new() -> Self {
        Self {
            event_queue: QueueCell::new(),
            status_queue: QueueCell::new(),
            base: AtomicUsize::new(0),
            slot: AtomicUsize::new(usize::MAX),
            supported_event_types: AtomicU32::new(0),
            events: IrqSpinLock::new(EventRing::EMPTY),
            event_buffer: IrqSpinLock::new(None),
        }
    }

    fn get_instance() -> &'static Self {
        static INPUT: VirtioInput = VirtioInput::new();
        &INPUT
    }

    pub(crate) fn initialize() -> bool {
        let input = Self::get_instance();
        if input.base.load(Ordering::Acquire) != 0 {
            return true;
        }

        let Some(candidate) = VirtioTransport::find(Self::DEVICE_ID) else {
            return false;
        };
        let Some(event_buffer) = DmaArena::allocate(Self::EVENT_SIZE, 4) else {
            return false;
        };
        let Ok(transport) = candidate.initialize(Self::DEVICE_ID) else {
            return false;
        };
        let Some(supported_event_types) = Self::query_event_types(&transport) else {
            return false;
        };
        if input.event_queue.lock().configure(&transport, 0).is_err()
            || input.status_queue.lock().configure(&transport, 1).is_err()
            || !transport.activate()
        {
            return false;
        }

        *input.event_buffer.lock() = Some(event_buffer);
        input
            .supported_event_types
            .store(supported_event_types, Ordering::Release);
        input.base.store(transport.base(), Ordering::Release);
        input.slot.store(transport.slot(), Ordering::Release);

        let mut events = input.events.lock();
        let notify = input.submit_event(&mut events);
        drop(events);
        if !notify {
            return false;
        }
        transport.notify(0);
        true
    }

    pub(crate) fn supports_event_type(event_type: u16) -> bool {
        if u32::from(event_type) >= u16::BITS {
            return false;
        }
        Self::get_instance()
            .supported_event_types
            .load(Ordering::Acquire)
            & (1u32 << event_type)
            != 0
    }

    pub(crate) fn read_event() -> Option<InputEvent> {
        let input = Self::get_instance();
        let base = input.base.load(Ordering::Acquire);
        if base == 0 {
            return None;
        }

        let mut events = input.events.lock();
        let event = events.pop();
        let notify = input.submit_event(&mut events);
        drop(events);
        if notify {
            if let Some(transport) = VirtioTransport::from_base(base) {
                transport.notify(0);
            }
        }
        event
    }

    pub(crate) fn handle_interrupt(pending: u64) {
        let input = Self::get_instance();
        let slot = input.slot.load(Ordering::Acquire);
        if slot >= 64 || pending & (1u64 << slot) == 0 {
            return;
        }
        let base = input.base.load(Ordering::Acquire);
        if base == 0 {
            return;
        }

        let mut events = input.events.lock();
        if let Some(expected) = events.active_head {
            if let Some(used) = input.event_queue.lock().poll_used() {
                events.active_head = None;
                if used.id == u32::from(expected) && used.length as usize >= Self::EVENT_SIZE {
                    let mut event_buffer = input.event_buffer.lock();
                    if let Some(buffer) = event_buffer.as_mut() {
                        let bytes = buffer.as_mut_slice();
                        events.push(InputEvent {
                            event_type: u16::from_le_bytes([bytes[0], bytes[1]]),
                            code: u16::from_le_bytes([bytes[2], bytes[3]]),
                            value: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
                        });
                    }
                }
            }
        }
        let notify = input.submit_event(&mut events);
        drop(events);
        if notify {
            if let Some(transport) = VirtioTransport::from_base(base) {
                transport.notify(0);
            }
        }
    }

    fn query_event_types(transport: &VirtioTransport) -> Option<u32> {
        let mut supported = 0u32;
        for event_type in 0..u16::BITS {
            transport.write_device_config_byte(
                Self::CONFIG_SELECT_OFFSET,
                Self::CONFIG_SELECT_EVENT_BITS,
            )?;
            transport.write_device_config_byte(Self::CONFIG_SUBSELECT_OFFSET, event_type as u8)?;
            let size = transport.read_device_config_byte(Self::CONFIG_SIZE_OFFSET)?;
            if size > Self::MAX_CONFIG_BITMAP_SIZE {
                return None;
            }
            if size != 0 {
                supported |= 1u32 << event_type;
            }
        }
        Some(supported)
    }

    fn submit_event(&self, events: &mut EventRing) -> bool {
        if events.active_head.is_some() || events.length == Self::EVENT_CAPACITY {
            return false;
        }
        let mut event_buffer = self.event_buffer.lock();
        let Some(buffer) = event_buffer.as_mut() else {
            return false;
        };
        buffer.as_mut_slice().fill(0);
        let descriptor = Descriptor::new(buffer.bus_address(), Self::EVENT_SIZE as u32, true);
        let Some(head) = self.event_queue.lock().submit(&[descriptor]) else {
            return false;
        };
        events.active_head = Some(head);
        DmaArena::sync_for_device();
        true
    }
}
