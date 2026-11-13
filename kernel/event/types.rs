/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU8, Ordering};

use super::{
    Event, EventFlags, EventFlagsApi, EventSignal, EventSignalApi, EventSourceId,
    EventSourceIdApi, EventSourceToken, EventSourceTokenApi,
};

impl EventSourceId {
    pub(crate) const fn new(id: u64) -> Self {
        Self(id)
    }
}

impl EventSourceIdApi for EventSourceId {
    fn new(id: u64) -> Self {
        Self(id)
    }

    fn raw(self) -> u64 {
        self.0
    }
}

impl EventSourceTokenApi for EventSourceToken {
    fn raw(self) -> u64 {
        (self.generation << 8) | self.index as u64
    }

    fn from_raw(raw: u64) -> Self {
        Self {
            index: (raw & 0xff) as usize,
            generation: raw >> 8,
        }
    }
}

impl EventFlagsApi for EventFlags {
    const NONE: Self = Self(0);
    const READABLE: Self = Self(1 << 0);
    const WRITABLE: Self = Self(1 << 1);
    const ERROR: Self = Self(1 << 2);
    const HANGUP: Self = Self(1 << 3);

    fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    fn from_bits(bits: u32) -> Option<Self> {
        if bits & !0x0f == 0 {
            Some(Self(bits as u8))
        } else {
            None
        }
    }

    fn bits(self) -> u8 {
        self.0
    }
}

impl EventFlags {
    pub(super) const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub(super) const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl EventSignalApi for EventSignal {
    fn new() -> Self {
        Self {
            readiness: Arc::new(AtomicU8::new(EventFlags::NONE.bits())),
        }
    }

    fn set_readiness(&self, readiness: EventFlags) {
        self.readiness.store(readiness.bits(), Ordering::Release);
    }

    fn source_id(&self) -> EventSourceId {
        <EventSourceId as EventSourceIdApi>::new(
            Arc::as_ptr(&self.readiness) as usize as u64,
        )
    }
}

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Timer(left), Self::Timer(right)) => Arc::ptr_eq(left, right),
            (Self::Completion(left), Self::Completion(right)) => left == right,
            (
                Self::Readiness {
                    source: left_source,
                    flags: left_flags,
                    data: left_data,
                },
                Self::Readiness {
                    source: right_source,
                    flags: right_flags,
                    data: right_data,
                },
            ) => {
                left_source.raw() == right_source.raw()
                    && left_flags == right_flags
                    && left_data == right_data
            }
            (Self::DeviceInterrupt { slot: left }, Self::DeviceInterrupt { slot: right }) => {
                left == right
            }
            _ => false,
        }
    }
}

impl Eq for Event {}