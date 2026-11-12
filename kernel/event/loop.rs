/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

/*
 * EventLoop: multiplex timers, completions, readiness, and device interrupts.
 *
 * Timers --------\
 * Completions ----+--> event queue --\
 * Readiness ----------> ready queue ---+--> fair poll --> Event
 * Device IRQs -------> pending bits --/
 */

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::ptr::NonNull;
use crate::kernel::error::Error;
use crate::kernel::lib::hashmap::HashMap;
use crate::kernel::lib::list::{LinkedList, ListNode};
use crate::kernel::event::timers::{Timer, TimerQueue};
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Completion {
    pub(crate) id: u64,
    pub(crate) status: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EventSourceId(u64);

impl EventSourceId {
    pub(crate) const fn new(id: u64) -> Self {
        Self(id)
    }

    pub(crate) const fn raw(self) -> u64 {
        self.0
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
    #[allow(dead_code)]
    pub(crate) const WRITABLE: Self = Self(1 << 1);
    #[allow(dead_code)]
    pub(crate) const ERROR: Self = Self(1 << 2);
    #[allow(dead_code)]
    pub(crate) const HANGUP: Self = Self(1 << 3);

    #[allow(dead_code)]
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

    pub(crate) const fn bits(self) -> u8 {
        self.0
    }

    const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone)]
pub(crate) struct EventSignal {
    readiness: Arc<AtomicU8>,
}

impl EventSignal {
    pub(crate) fn new() -> Self {
        Self {
            readiness: Arc::new(AtomicU8::new(EventFlags::NONE.bits())),
        }
    }

    pub(crate) fn set_readiness(&self, readiness: EventFlags) {
        self.readiness.store(readiness.bits(), Ordering::Release);
    }

    pub(crate) fn source_id(&self) -> EventSourceId {
        EventSourceId::new(Arc::as_ptr(&self.readiness) as usize as u64)
    }

    fn readiness(&self) -> EventFlags {
        EventFlags::from_bits(u32::from(self.readiness.load(Ordering::Acquire)))
            .unwrap_or(EventFlags::NONE)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Event {
    Timer(Arc<Timer>),
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
                left_source == right_source
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

#[derive(Clone)]
struct SourceWatch {
    source: EventSourceId,
    interest: EventFlags,
    readiness: EventFlags,
    data: u64,
    generation: u64,
    ready_node: Option<NonNull<ListNode<usize>>>,
    signal: Option<EventSignal>,
    active: bool,
}

impl SourceWatch {
    const EMPTY: Self = Self {
        source: EventSourceId(0),
        interest: EventFlags::NONE,
        readiness: EventFlags::NONE,
        data: 0,
        generation: 0,
        ready_node: None,
        signal: None,
        active: false,
    };
}

pub(crate) struct EventLoop<const N: usize> {
    timers: TimerQueue<N>,
    events: LinkedList<Event>,
    ready_sources: LinkedList<usize>,
    sources: [SourceWatch; N],
    source_indices: HashMap<u64, usize>,
    free_sources: [usize; N],
    free_source_count: usize,
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
        let mut free_sources = [0; N];
        let mut index = 0;
        while index < N {
            free_sources[index] = index;
            index += 1;
        }
        Self {
            timers: TimerQueue::new(),
            events: LinkedList::new(),
            ready_sources: LinkedList::new(),
            sources: [const { SourceWatch::EMPTY }; N],
            source_indices: HashMap::new(),
            free_sources,
            free_source_count: N,
            pending_device_interrupts: 0,
            next_queue: 0,
        }
    }

    pub(crate) fn schedule_timer(
        &mut self,
        deadline_ns: u64,
    ) -> Option<Arc<Timer>> {
        self.timers.schedule(deadline_ns)
    }

    pub(crate) fn timer_by_address(&self, address: u64) -> Option<Arc<Timer>> {
        self.timers.timer_by_address(address)
    }

    pub(crate) fn cancel_timer(&mut self, timer: &Arc<Timer>) -> bool {
        self.timers.cancel(timer)
    }

    pub(crate) fn register(
        &mut self,
        source: EventSourceId,
        interest: EventFlags,
        data: u64,
    ) -> Result<EventSourceToken, Error> {
        self.register_inner(source, interest, data, None)
    }

    pub(crate) fn register_signal(
        &mut self,
        source: EventSourceId,
        interest: EventFlags,
        data: u64,
        signal: EventSignal,
    ) -> Result<EventSourceToken, Error> {
        self.register_inner(source, interest, data, Some(signal))
    }

    fn register_inner(
        &mut self,
        source: EventSourceId,
        interest: EventFlags,
        data: u64,
        signal: Option<EventSignal>,
    ) -> Result<EventSourceToken, Error> {
        if self.source_indices.contains_key(&source.0) {
            return Err(Error::Exist);
        }
        if self.free_source_count == 0 {
            return Err(Error::NoBufs);
        }
        let index = self.free_sources[self.free_source_count - 1];
        let generation = self.sources[index].generation.wrapping_add(1).max(1);
        match self.source_indices.insert(source.0, index) {
            Ok(None) => {}
            Ok(Some(previous)) => {
                let _ = self.source_indices.insert(source.0, previous);
                return Err(Error::Exist);
            }
            Err(_) => return Err(Error::NoMem),
        }
        self.free_source_count -= 1;
        self.sources[index] = SourceWatch {
            source,
            interest,
            readiness: signal
                .as_ref()
                .map_or(EventFlags::NONE, EventSignal::readiness),
            data,
            generation,
            ready_node: None,
            signal,
            active: true,
        };
        self.queue_ready(index);
        Ok(EventSourceToken { index, generation })
    }

    pub(crate) fn modify(
        &mut self,
        token: EventSourceToken,
        interest: EventFlags,
        data: u64,
    ) -> Result<(), Error> {
        let index = self.source_index(token)?;
        self.sources[index].interest = interest;
        self.sources[index].data = data;
        self.queue_ready(index);
        Ok(())
    }

    pub(crate) fn deregister(&mut self, token: EventSourceToken) -> Result<(), Error> {
        let index = self.source_index(token)?;
        if let Some(node) = self.sources[index].ready_node.take() {
            // SAFETY: this node is stored by its source watch while it is linked in ready_sources.
            let removed = unsafe { ListNode::remove(&mut self.ready_sources, node) };
            debug_assert!(removed.is_some());
            drop(removed);
        }
        let source_id = self.sources[index].source.0;
        let generation = self.sources[index].generation;
        self.sources[index] = SourceWatch {
            generation,
            ..SourceWatch::EMPTY
        };
        self.source_indices.remove(&source_id);
        self.free_sources[self.free_source_count] = index;
        self.free_source_count += 1;
        Ok(())
    }

    pub(crate) fn set_readiness(
        &mut self,
        token: EventSourceToken,
        readiness: EventFlags,
    ) -> Result<(), Error> {
        let index = self.source_index(token)?;
        self.sources[index].readiness = readiness;
        self.queue_ready(index);
        Ok(())
    }

    pub(crate) fn complete(&mut self, completion: Completion) -> Result<(), Completion> {
        self.events
            .push_back(Box::new(ListNode::new(Event::Completion(completion))));
        Ok(())
    }

    pub(crate) fn next_deadline(&self) -> Option<u64> {
        self.timers.next_deadline()
    }

    pub(crate) fn wait(&mut self, now_ns: u64) -> Option<Event> {
        self.queue_expired_timers(now_ns);
        self.refresh_signals();
        self.pending_device_interrupts |=
            Self::pending_interrupt_source().swap(0, Ordering::AcqRel);
        for _ in 0..3 {
            let queue = self.next_queue;
            self.next_queue = (self.next_queue + 1) % 3;
            let event = match queue {
                0 => self.next_readiness(),
                1 => self.events.pop_front().map(|node| node.into_value()),
                _ => self.next_device_interrupt(),
            };
            if event.is_some() {
                return event;
            }
        }
        None
    }

    pub(crate) fn poll_readiness(&mut self) -> Option<Event> {
        self.refresh_signals();
        self.next_readiness()
    }

    fn next_device_interrupt(&mut self) -> Option<Event> {
        if self.pending_device_interrupts == 0 {
            return None;
        }
        let slot = self.pending_device_interrupts.trailing_zeros();
        self.pending_device_interrupts &= !(1u64 << slot);
        Some(Event::DeviceInterrupt { slot: slot as u8 })
    }

    fn source_index(&self, token: EventSourceToken) -> Result<usize, Error> {
        let Some(watch) = self.sources.get(token.index) else {
            return Err(Error::NoEnt);
        };
        if !watch.active || watch.generation != token.generation {
            return Err(Error::NoEnt);
        }
        Ok(token.index)
    }

    fn queue_expired_timers(&mut self, now_ns: u64) {
        while let Some(timer) = self.timers.pop_expired(now_ns) {
            self.events
                .push_back(Box::new(ListNode::new(Event::Timer(timer))));
        }
    }

    fn queue_ready(&mut self, index: usize) {
        let watch = self.sources[index].clone();
        if !watch.active
            || watch.ready_node.is_some()
            || watch.readiness.intersection(watch.interest).is_empty()
        {
            return;
        }
        let mut node = Box::new(ListNode::new(index));
        let node_pointer = NonNull::from(node.as_mut());
        self.ready_sources.push_back(node);
        self.sources[index].ready_node = Some(node_pointer);
    }

    fn refresh_signals(&mut self) {
        for index in 0..N {
            let Some(signal) = self.sources[index].signal.clone() else {
                continue;
            };
            let readiness = signal.readiness();
            if self.sources[index].readiness != readiness {
                self.sources[index].readiness = readiness;
                self.queue_ready(index);
            }
        }
    }

    fn next_readiness(&mut self) -> Option<Event> {
        while let Some(node) = self.ready_sources.pop_front() {
            let index = node.into_value();
            let event = {
                let watch = &mut self.sources[index];
                watch.ready_node = None;
                if !watch.active {
                    let generation = watch.generation;
                    *watch = SourceWatch {
                        generation,
                        ..SourceWatch::EMPTY
                    };
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
#[path = "../../tests/kernel/event/loop.rs"]
mod tests;
