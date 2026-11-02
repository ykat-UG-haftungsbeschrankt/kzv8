/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::event::completion::{Completion, CompletionQueue};
use crate::kernel::event::timers::{TimerId, TimerQueue};
use core::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EventSourceId(u64);

impl EventSourceId {
    pub(crate) const fn new(id: u64) -> Self {
        Self(id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EventSourceToken {
    index: usize,
    generation: u64,
}

impl EventSourceToken {
    pub(crate) const fn raw(self) -> u64 {
        (self.generation << 8) | self.index as u64
    }

    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self {
            index: (raw & 0xff) as usize,
            generation: raw >> 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EventFlags(u8);

impl EventFlags {
    pub(crate) const NONE: Self = Self(0);
    pub(crate) const READABLE: Self = Self(1 << 0);
    pub(crate) const WRITABLE: Self = Self(1 << 1);
    pub(crate) const ERROR: Self = Self(1 << 2);
    pub(crate) const HANGUP: Self = Self(1 << 3);

    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) const fn from_bits(bits: u32) -> Option<Self> {
        if bits & !0x0f == 0 {
            Some(Self(bits as u8))
        } else {
            None
        }
    }

    const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Event {
    Timer(TimerId),
    Completion(Completion),
    Readiness {
        source: EventSourceId,
        flags: EventFlags,
        data: u64,
    },
    DeviceInterrupt {
        slot: u8,
    },
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RawEvent {
    pub(crate) kind: u32,
    pub(crate) flags: u32,
    pub(crate) id: u64,
    pub(crate) status: i32,
    pub(crate) reserved: u32,
    pub(crate) data: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegistrationError {
    AlreadyRegistered,
    NotRegistered,
    Full,
}

#[derive(Clone, Copy)]
struct SourceWatch {
    source: EventSourceId,
    interest: EventFlags,
    readiness: EventFlags,
    data: u64,
    generation: u64,
    queued: bool,
    active: bool,
}

impl SourceWatch {
    const EMPTY: Self = Self {
        source: EventSourceId(0),
        interest: EventFlags::NONE,
        readiness: EventFlags::NONE,
        data: 0,
        generation: 0,
        queued: false,
        active: false,
    };
}

pub(crate) struct EventLoop<const N: usize> {
    timers: TimerQueue<N>,
    events: CompletionQueue<Event, N>,
    ready_sources: CompletionQueue<usize, N>,
    sources: [SourceWatch; N],
    pending_device_interrupts: u64,
    next_queue: usize,
}

impl<const N: usize> EventLoop<N> {
    fn pending_interrupt_source() -> &'static AtomicU64 {
        static PENDING_DEVICE_INTERRUPTS: AtomicU64 = AtomicU64::new(0);
        &PENDING_DEVICE_INTERRUPTS
    }

    pub(crate) fn record_device_interrupts(interrupts: u64) {
        Self::pending_interrupt_source().fetch_or(interrupts, Ordering::Release);
    }

    pub(crate) const fn new() -> Self {
        Self {
            timers: TimerQueue::new(),
            events: CompletionQueue::new(),
            ready_sources: CompletionQueue::new(),
            sources: [SourceWatch::EMPTY; N],
            pending_device_interrupts: 0,
            next_queue: 0,
        }
    }

    pub(crate) fn schedule_timer(&mut self, deadline_ns: u64) -> Option<TimerId> {
        self.timers.schedule(deadline_ns)
    }

    pub(crate) fn cancel_timer(&mut self, timer: TimerId) -> bool {
        self.timers.cancel(timer)
    }

    pub(crate) fn register(
        &mut self,
        source: EventSourceId,
        interest: EventFlags,
        data: u64,
    ) -> Result<EventSourceToken, RegistrationError> {
        if self
            .sources
            .iter()
            .any(|watch| watch.active && watch.source == source)
        {
            return Err(RegistrationError::AlreadyRegistered);
        }
        let Some(index) = self
            .sources
            .iter()
            .position(|watch| !watch.active && !watch.queued)
        else {
            return Err(RegistrationError::Full);
        };
        let generation = self.sources[index].generation.wrapping_add(1).max(1);
        self.sources[index] = SourceWatch {
            source,
            interest,
            readiness: EventFlags::NONE,
            data,
            generation,
            queued: false,
            active: true,
        };
        Ok(EventSourceToken { index, generation })
    }

    pub(crate) fn modify(
        &mut self,
        token: EventSourceToken,
        interest: EventFlags,
        data: u64,
    ) -> Result<(), RegistrationError> {
        let index = self.source_index(token)?;
        self.sources[index].interest = interest;
        self.sources[index].data = data;
        self.queue_ready(index);
        Ok(())
    }

    pub(crate) fn deregister(&mut self, token: EventSourceToken) -> Result<(), RegistrationError> {
        let index = self.source_index(token)?;
        self.ready_sources
            .retain(|ready_index| ready_index != index);
        let generation = self.sources[index].generation;
        self.sources[index] = SourceWatch {
            generation,
            ..SourceWatch::EMPTY
        };
        Ok(())
    }

    pub(crate) fn set_readiness(
        &mut self,
        token: EventSourceToken,
        readiness: EventFlags,
    ) -> Result<(), RegistrationError> {
        let index = self.source_index(token)?;
        self.sources[index].readiness = readiness;
        self.queue_ready(index);
        Ok(())
    }

    pub(crate) fn complete(&mut self, completion: Completion) -> Result<(), Completion> {
        self.events
            .push(Event::Completion(completion))
            .map_err(|event| match event {
                Event::Completion(completion) => completion,
                Event::Timer(_) | Event::Readiness { .. } | Event::DeviceInterrupt { .. } => {
                    unreachable!()
                }
            })
    }

    pub(crate) fn next_deadline(&self) -> Option<u64> {
        self.timers.next_deadline()
    }

    pub(crate) fn wait(&mut self, now_ns: u64) -> Option<Event> {
        self.queue_expired_timers(now_ns);
        self.pending_device_interrupts |=
            Self::pending_interrupt_source().swap(0, Ordering::AcqRel);
        for _ in 0..3 {
            let queue = self.next_queue;
            self.next_queue = (self.next_queue + 1) % 3;
            let event = match queue {
                0 => self.next_readiness(),
                1 => self.events.pop(),
                _ => self.next_device_interrupt(),
            };
            if event.is_some() {
                return event;
            }
        }
        None
    }

    pub(crate) fn poll_raw(&mut self, now_ns: u64) -> Option<RawEvent> {
        self.wait(now_ns).map(|event| match event {
            Event::Timer(timer) => RawEvent {
                kind: 1,
                id: timer.raw(),
                ..RawEvent::default()
            },
            Event::Completion(completion) => RawEvent {
                kind: 2,
                id: completion.id,
                status: completion.status,
                ..RawEvent::default()
            },
            Event::Readiness {
                source,
                flags,
                data,
            } => RawEvent {
                kind: 3,
                flags: u32::from(flags.0),
                id: source.0,
                data,
                ..RawEvent::default()
            },
            Event::DeviceInterrupt { slot } => RawEvent {
                kind: 4,
                id: u64::from(slot),
                ..RawEvent::default()
            },
        })
    }

    fn next_device_interrupt(&mut self) -> Option<Event> {
        if self.pending_device_interrupts == 0 {
            return None;
        }
        let slot = self.pending_device_interrupts.trailing_zeros();
        self.pending_device_interrupts &= !(1u64 << slot);
        Some(Event::DeviceInterrupt { slot: slot as u8 })
    }

    fn source_index(&self, token: EventSourceToken) -> Result<usize, RegistrationError> {
        let Some(watch) = self.sources.get(token.index) else {
            return Err(RegistrationError::NotRegistered);
        };
        if !watch.active || watch.generation != token.generation {
            return Err(RegistrationError::NotRegistered);
        }
        Ok(token.index)
    }

    fn queue_expired_timers(&mut self, now_ns: u64) {
        while self.events.len() < N {
            let Some(timer) = self.timers.pop_expired(now_ns) else {
                break;
            };
            if self.events.push(Event::Timer(timer)).is_err() {
                break;
            }
        }
    }

    fn queue_ready(&mut self, index: usize) {
        let watch = self.sources[index];
        if !watch.active || watch.queued || watch.readiness.intersection(watch.interest).is_empty()
        {
            return;
        }
        if self.ready_sources.push(index).is_ok() {
            self.sources[index].queued = true;
        }
    }

    fn next_readiness(&mut self) -> Option<Event> {
        while let Some(index) = self.ready_sources.pop() {
            let event = {
                let watch = &mut self.sources[index];
                watch.queued = false;
                if !watch.active {
                    *watch = SourceWatch::EMPTY;
                    continue;
                }
                let flags = watch.readiness.intersection(watch.interest);
                if flags.is_empty() {
                    continue;
                }
                Event::Readiness {
                    source: watch.source,
                    flags,
                    data: watch.data,
                }
            };
            self.queue_ready(index);
            return Some(event);
        }
        None
    }
}
