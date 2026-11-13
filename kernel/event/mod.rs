/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;
use core::sync::atomic::AtomicU8;

use crate::kernel::error::Error;

mod r#loop;
mod timers;
mod types;

pub(crate) use r#loop::EventLoop;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Completion {
    pub(crate) id: u64,
    pub(crate) status: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EventSourceId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EventSourceToken {
    index: usize,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EventFlags(u8);

#[derive(Clone)]
pub(crate) struct EventSignal {
    readiness: Arc<AtomicU8>,
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

#[derive(Debug)]
pub(crate) struct Timer {
    deadline_ns: u64,
}

pub(crate) trait EventSourceIdApi: Sized {
    fn new(id: u64) -> Self;
    fn raw(self) -> u64;
}

pub(crate) trait EventSourceTokenApi: Sized {
    fn raw(self) -> u64;
    fn from_raw(raw: u64) -> Self;
}

pub(crate) trait EventFlagsApi: Sized {
    const NONE: Self;
    const READABLE: Self;
    #[allow(dead_code)]
    const WRITABLE: Self;
    #[allow(dead_code)]
    const ERROR: Self;
    #[allow(dead_code)]
    const HANGUP: Self;

    #[allow(dead_code)]
    fn union(self, other: Self) -> Self;
    fn from_bits(bits: u32) -> Option<Self>;
    fn bits(self) -> u8;
}

pub(crate) trait EventSignalApi {
    fn new() -> Self
    where
        Self: Sized;

    fn set_readiness(&self, readiness: EventFlags);
    fn source_id(&self) -> EventSourceId;
}

pub(crate) trait EventLoopApi {
    fn new() -> Self
    where
        Self: Sized;

    fn record_device_interrupts(interrupts: u64)
    where
        Self: Sized;

    fn schedule_timer(&mut self, deadline_ns: u64) -> Option<Arc<Timer>>;
    fn timer_by_address(&self, address: u64) -> Option<Arc<Timer>>;
    fn cancel_timer(&mut self, timer: &Arc<Timer>) -> bool;
    fn register(
        &mut self,
        source: EventSourceId,
        interest: EventFlags,
        data: u64,
    ) -> Result<EventSourceToken, Error>;
    fn register_signal(
        &mut self,
        source: EventSourceId,
        interest: EventFlags,
        data: u64,
        signal: EventSignal,
    ) -> Result<EventSourceToken, Error>;
    fn modify(
        &mut self,
        token: EventSourceToken,
        interest: EventFlags,
        data: u64,
    ) -> Result<(), Error>;
    fn deregister(&mut self, token: EventSourceToken) -> Result<(), Error>;
    fn set_readiness(
        &mut self,
        token: EventSourceToken,
        readiness: EventFlags,
    ) -> Result<(), Error>;
    fn complete(&mut self, completion: Completion) -> Result<(), Completion>;
    fn next_deadline(&self) -> Option<u64>;
    fn wait(&mut self, now_ns: u64) -> Option<Event>;
    fn poll_readiness(&mut self) -> Option<Event>;
}
