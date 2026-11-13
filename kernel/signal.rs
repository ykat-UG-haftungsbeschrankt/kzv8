/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::error::Error;

pub(crate) const MAX_SIGNAL_NUMBER: u8 = 63;
pub(crate) const REALTIME_SIGNAL_MIN: u8 = 34;
pub(crate) const SIGINT: u8 = 2;
pub(crate) const SIGQUIT: u8 = 3;
pub(crate) const SIGKILL: u8 = 9;
pub(crate) const SIGSTOP: u8 = 19;
pub(crate) const SIGCONT: u8 = 18;
pub(crate) const SIGTSTP: u8 = 20;
pub(crate) const SIGTTIN: u8 = 21;
pub(crate) const SIGTTOU: u8 = 22;
pub(crate) const SIGALRM: u8 = 14;
pub(crate) const SIGCHLD: u8 = 17;
pub(crate) const SI_QUEUE: i32 = -1;
pub(crate) const SI_TIMER: i32 = -2;
pub(crate) const SA_NOCLDSTOP: u32 = 0x0000_0001;
pub(crate) const CLD_EXITED: i32 = 1;
pub(crate) const CLD_KILLED: i32 = 2;
pub(crate) const CLD_STOPPED: i32 = 5;
pub(crate) const CLD_CONTINUED: i32 = 6;
pub(crate) const SIGNAL_QUEUE_CAPACITY: usize = 32;

const VALID_SIGNAL_MASK: u64 = u64::MAX >> 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SignalSet(u64);

impl SignalSet {
    pub(crate) const EMPTY: Self = Self(0);
    pub(crate) const ALL: Self = Self(VALID_SIGNAL_MASK);

    pub(crate) const fn from_bits(bits: u64) -> Self {
        Self(bits & VALID_SIGNAL_MASK)
    }

    pub(crate) const fn bits(self) -> u64 {
        self.0
    }

    pub(crate) const fn from_signal(signal_number: u8) -> Self {
        if signal_number == 0 || signal_number > MAX_SIGNAL_NUMBER {
            Self::EMPTY
        } else {
            Self(1u64 << (signal_number - 1))
        }
    }

    pub(crate) const fn contains(self, signal_number: u8) -> bool {
        signal_number != 0
            && signal_number <= MAX_SIGNAL_NUMBER
            && self.0 & (1u64 << (signal_number - 1)) != 0
    }

    pub(crate) const fn insert(self, signal_number: u8) -> Self {
        Self::from_bits(self.0 | Self::from_signal(signal_number).0)
    }

    pub(crate) const fn remove(self, signal_number: u8) -> Self {
        Self::from_bits(self.0 & !Self::from_signal(signal_number).0)
    }

    pub(crate) const fn union(self, other: Self) -> Self {
        Self::from_bits(self.0 | other.0)
    }

    pub(crate) const fn intersection(self, other: Self) -> Self {
        Self::from_bits(self.0 & other.0)
    }

    pub(crate) const fn difference(self, other: Self) -> Self {
        Self::from_bits(self.0 & !other.0)
    }

    pub(crate) const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(crate) const fn without_uncatchable(self) -> Self {
        self.remove(SIGKILL).remove(SIGSTOP)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SignalInfo {
    pub(crate) signal_number: u8,
    pub(crate) code: i32,
    pub(crate) sender_pid: i32,
    pub(crate) sender_uid: u32,
    pub(crate) value: usize,
    pub(crate) status: i32,
}

impl SignalInfo {
    pub(crate) const fn new(
        signal_number: u8,
        code: i32,
        sender_pid: i32,
        sender_uid: u32,
        value: usize,
        status: i32,
    ) -> Self {
        Self {
            signal_number,
            code,
            sender_pid,
            sender_uid,
            value,
            status,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SignalAction {
    pub(crate) handler: usize,
    pub(crate) mask: SignalSet,
    pub(crate) flags: u32,
}

impl SignalAction {
    pub(crate) const DEFAULT: Self = Self {
        handler: 0,
        mask: SignalSet::EMPTY,
        flags: 0,
    };

    pub(crate) const fn is_default(self) -> bool {
        self.handler == 0
    }

    pub(crate) const fn is_ignored(self) -> bool {
        self.handler == 1
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SignalQueue {
    entries: [Option<SignalInfo>; SIGNAL_QUEUE_CAPACITY],
    length: usize,
}

impl SignalQueue {
    pub(crate) const fn new() -> Self {
        Self {
            entries: [None; SIGNAL_QUEUE_CAPACITY],
            length: 0,
        }
    }

    pub(crate) fn push(&mut self, information: SignalInfo) -> Result<(), Error> {
        if information.signal_number < REALTIME_SIGNAL_MIN
            && self.entries[..self.length]
                .iter()
                .flatten()
                .any(|pending| pending.signal_number == information.signal_number)
        {
            return Ok(());
        }
        if self.length == SIGNAL_QUEUE_CAPACITY {
            return Err(Error::NoSpc);
        }
        self.entries[self.length] = Some(information);
        self.length += 1;
        Ok(())
    }

    pub(crate) fn take(&mut self, signals: SignalSet) -> Option<SignalInfo> {
        let mut selected = None;
        let mut selected_signal = u8::MAX;
        for (index, pending) in self.entries[..self.length].iter().enumerate() {
            let Some(pending) = pending else {
                continue;
            };
            if signals.contains(pending.signal_number) && pending.signal_number < selected_signal {
                selected = Some(index);
                selected_signal = pending.signal_number;
            }
        }
        let selected = selected?;
        let information = self.entries[selected].take()?;
        for index in selected..self.length - 1 {
            self.entries[index] = self.entries[index + 1];
        }
        self.length -= 1;
        self.entries[self.length] = None;
        Some(information)
    }

    pub(crate) fn remove_signal(&mut self, signal_number: u8) {
        let mut index = 0;
        while index < self.length {
            if self.entries[index].is_some_and(|pending| pending.signal_number == signal_number) {
                for next in index..self.length - 1 {
                    self.entries[next] = self.entries[next + 1];
                }
                self.length -= 1;
                self.entries[self.length] = None;
            } else {
                index += 1;
            }
        }
    }

    pub(crate) fn pending_set(&self) -> SignalSet {
        self.entries[..self.length]
            .iter()
            .flatten()
            .fold(SignalSet::EMPTY, |set, pending| {
                set.insert(pending.signal_number)
            })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SignalState {
    actions: [SignalAction; MAX_SIGNAL_NUMBER as usize + 1],
    pub(crate) pending: SignalQueue,
    alarm_deadline_ns: Option<u64>,
    terminated_by_signal: Option<u8>,
    stopped_by_signal: Option<u8>,
}

impl SignalState {
    pub(crate) const fn new() -> Self {
        Self {
            actions: [SignalAction::DEFAULT; MAX_SIGNAL_NUMBER as usize + 1],
            pending: SignalQueue::new(),
            alarm_deadline_ns: None,
            terminated_by_signal: None,
            stopped_by_signal: None,
        }
    }

    pub(crate) fn for_spawn(&self, default_signals: SignalSet) -> Self {
        let mut state = *self;
        state.pending = SignalQueue::new();
        state.alarm_deadline_ns = None;
        state.terminated_by_signal = None;
        state.stopped_by_signal = None;
        for signal_number in 1..=MAX_SIGNAL_NUMBER {
            if default_signals.contains(signal_number) {
                state.actions[signal_number as usize] = SignalAction::DEFAULT;
            }
        }
        state
    }

    pub(crate) fn reset_caught_actions_for_exec(&mut self) {
        for action in self.actions.iter_mut().skip(1) {
            if !action.is_ignored() {
                *action = SignalAction::DEFAULT;
            }
        }
    }

    pub(crate) fn action(&self, signal_number: u8) -> Option<SignalAction> {
        (signal_number != 0 && signal_number <= MAX_SIGNAL_NUMBER)
            .then(|| self.actions[signal_number as usize])
    }

    pub(crate) fn set_action(
        &mut self,
        signal_number: u8,
        action: SignalAction,
    ) -> Result<SignalAction, Error> {
        if signal_number == 0
            || signal_number > MAX_SIGNAL_NUMBER
            || matches!(signal_number, SIGKILL | SIGSTOP)
            || action.handler == usize::MAX
        {
            return Err(Error::Inval);
        }
        let slot = &mut self.actions[signal_number as usize];
        let previous = *slot;
        *slot = action;
        if action.is_ignored() {
            self.pending.remove_signal(signal_number);
        }
        Ok(previous)
    }

    pub(crate) fn set_alarm(&mut self, now_ns: u64, seconds: u32) -> u32 {
        let previous = self.alarm_deadline_ns.map_or(0, |deadline| {
            deadline
                .saturating_sub(now_ns)
                .saturating_add(999_999_999)
                .checked_div(1_000_000_000)
                .unwrap_or(0)
                .min(u32::MAX as u64) as u32
        });
        self.alarm_deadline_ns = if seconds == 0 {
            None
        } else {
            Some(now_ns.saturating_add(seconds as u64 * 1_000_000_000))
        };
        previous
    }

    pub(crate) fn expire_alarm(&mut self, now_ns: u64) -> bool {
        let Some(deadline) = self.alarm_deadline_ns else {
            return false;
        };
        if deadline > now_ns {
            return false;
        }
        self.alarm_deadline_ns = None;
        self.pending
            .push(SignalInfo::new(SIGALRM, SI_TIMER, 0, 0, 0, 0))
            .is_ok()
    }

    pub(crate) fn mark_terminated_by_signal(&mut self, signal_number: u8) -> bool {
        if self.terminated_by_signal.is_some() {
            return false;
        }
        self.terminated_by_signal = Some(signal_number);
        self.stopped_by_signal = None;
        true
    }

    pub(crate) const fn termination_signal(&self) -> Option<u8> {
        self.terminated_by_signal
    }

    pub(crate) fn mark_stopped_by_signal(&mut self, signal_number: u8) -> bool {
        if self.terminated_by_signal.is_some() || self.stopped_by_signal.is_some() {
            return false;
        }
        self.stopped_by_signal = Some(signal_number);
        true
    }

    pub(crate) fn resume(&mut self) -> bool {
        self.stopped_by_signal.take().is_some()
    }

    pub(crate) const fn stopped_by_signal(&self) -> Option<u8> {
        self.stopped_by_signal
    }
}

#[cfg(test)]
#[path = "../tests/kernel/signal.rs"]
mod tests;