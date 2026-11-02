/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::event_completion::{Completion, CompletionQueue};
use crate::event_timers::{TimerId, TimerQueue};
use core::sync::atomic::{AtomicU64, Ordering};

static PENDING_DEVICE_INTERRUPTS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn record_device_interrupts(interrupts: u64) {
    PENDING_DEVICE_INTERRUPTS.fetch_or(interrupts, Ordering::Release);
}

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
    DeviceInterrupt { slot: u8 },
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
        self.ready_sources.retain(|ready_index| ready_index != index);
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
        self.pending_device_interrupts |= PENDING_DEVICE_INTERRUPTS.swap(0, Ordering::AcqRel);
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
        if !watch.active
            || watch.queued
            || watch.readiness.intersection(watch.interest).is_empty()
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

#[cfg(test)]
mod tests {
    use super::{Event, EventFlags, EventLoop, EventSourceId, RegistrationError};
    use crate::event_completion::Completion;

    #[test]
    fn readiness_is_filtered_and_level_triggered() {
        let source = EventSourceId::new(4);
        let mut events = EventLoop::<2>::new();
        let token = events.register(source, EventFlags::READABLE, 17).unwrap();
        events
            .set_readiness(token, EventFlags::READABLE.union(EventFlags::WRITABLE))
            .unwrap();

        let ready = Event::Readiness {
            source,
            flags: EventFlags::READABLE,
            data: 17,
        };
        assert_eq!(events.wait(0), Some(ready));
        assert_eq!(events.wait(0), Some(ready));
        events.set_readiness(token, EventFlags::NONE).unwrap();
        assert_eq!(events.wait(0), None);
    }

    #[test]
    fn modify_and_deregister_update_queued_readiness() {
        let source = EventSourceId::new(8);
        let mut events = EventLoop::<1>::new();
        let token = events.register(source, EventFlags::READABLE, 1).unwrap();
        events
            .set_readiness(token, EventFlags::READABLE)
            .unwrap();
        events
            .modify(token, EventFlags::WRITABLE, 2)
            .unwrap();
        assert_eq!(events.wait(0), None);

        events
            .set_readiness(token, EventFlags::WRITABLE)
            .unwrap();
        assert_eq!(
            events.wait(0),
            Some(Event::Readiness {
                source,
                flags: EventFlags::WRITABLE,
                data: 2,
            })
        );
        events.deregister(token).unwrap();
        assert_eq!(
            events.set_readiness(token, EventFlags::READABLE),
            Err(RegistrationError::NotRegistered)
        );
        let replacement = events
            .register(source, EventFlags::READABLE, 3)
            .unwrap();
        assert_ne!(token, replacement);
    }

    #[test]
    fn timers_and_completions_share_the_wait_stream() {
        let mut events = EventLoop::<2>::new();
        let timer = events.schedule_timer(10).unwrap();
        let completion = Completion { id: 2, status: 0 };
        assert_eq!(events.complete(completion), Ok(()));
        assert_eq!(events.wait(9), Some(Event::Completion(completion)));
        assert_eq!(events.wait(10), Some(Event::Timer(timer)));
        assert!(events
            .register(EventSourceId::new(1), EventFlags::NONE, 0)
            .is_ok());
        assert_eq!(
            events.register(EventSourceId::new(1), EventFlags::NONE, 0),
            Err(RegistrationError::AlreadyRegistered)
        );
    }

    #[test]
    fn device_interrupts_are_coalesced_into_events() {
        let mut events = EventLoop::<1>::new();
        events.pending_device_interrupts = (1 << 5) | (1 << 9);

        assert_eq!(events.wait(0), Some(Event::DeviceInterrupt { slot: 5 }));
        assert_eq!(events.wait(0), Some(Event::DeviceInterrupt { slot: 9 }));
        assert_eq!(events.wait(0), None);
    }
}
