/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::lib::btree::BinaryTree;

const NO_INDEX: usize = usize::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TimerId {
    address: u64,
}

impl TimerId {
    pub(crate) const fn raw(self) -> u64 {
        self.address
    }

    pub(crate) const fn from_raw(raw: u64) -> Self {
        Self { address: raw }
    }
}

#[derive(Clone, Copy)]
struct Timer {
    deadline_ns: u64,
    active: bool,
    next_free: usize,
}

impl Timer {
    const EMPTY: Self = Self {
        deadline_ns: 0,
        active: false,
        next_free: NO_INDEX,
    };
}

pub(crate) struct TimerQueue<const N: usize> {
    timers: [Timer; N],
    deadlines: BinaryTree<(u64, usize), usize>,
    free_head: usize,
}

impl<const N: usize> TimerQueue<N> {
    pub(crate) const fn new() -> Self {
        assert!(N > 0);
        let mut timers = [Timer::EMPTY; N];
        let mut index = 0;
        while index < N {
            timers[index].next_free = if index + 1 < N { index + 1 } else { NO_INDEX };
            index += 1;
        }
        Self {
            timers,
            deadlines: BinaryTree::new(),
            free_head: 0,
        }
    }

    fn timer_id(&self, index: usize) -> TimerId {
        TimerId {
            address: (&self.timers[index] as *const Timer as usize) as u64,
        }
    }

    fn timer_index(&self, id: TimerId) -> Option<usize> {
        let address = usize::try_from(id.address).ok()?;
        let base = self.timers.as_ptr() as usize;
        let offset = address.checked_sub(base)?;
        let timer_size = core::mem::size_of::<Timer>();
        if offset % timer_size != 0 {
            return None;
        }
        let index = offset / timer_size;
        (index < N).then_some(index)
    }

    pub(crate) fn schedule(&mut self, deadline_ns: u64) -> Option<TimerId> {
        let index = self.free_head;
        if index == NO_INDEX {
            return None;
        }
        self.free_head = self.timers[index].next_free;

        self.timers[index] = Timer {
            deadline_ns,
            active: true,
            next_free: NO_INDEX,
        };
        let id = self.timer_id(index);
        let previous = self.deadlines.insert((deadline_ns, index), index);
        debug_assert!(previous.is_none());
        Some(id)
    }

    pub(crate) fn cancel(&mut self, id: TimerId) -> bool {
        let Some(index) = self.timer_index(id) else {
            return false;
        };
        let timer = &self.timers[index];
        if !timer.active {
            return false;
        }

        let deadline_ns = timer.deadline_ns;
        if self.deadlines.remove(&(deadline_ns, index)).is_none() {
            return false;
        }
        self.release(index);
        true
    }

    pub(crate) fn pop_expired(&mut self, now_ns: u64) -> Option<TimerId> {
        let ((deadline_ns, _), _) = self.deadlines.first()?;
        if *deadline_ns > now_ns {
            return None;
        }

        let ((_, index), _) = self.deadlines.take_min()?;
        let id = self.timer_id(index);
        self.release(index);
        Some(id)
    }

    pub(crate) fn next_deadline(&self) -> Option<u64> {
        self.deadlines.first().map(|((deadline_ns, _), _)| *deadline_ns)
    }

    fn release(&mut self, index: usize) {
        self.timers[index].active = false;
        self.timers[index].next_free = self.free_head;
        self.free_head = index;
    }
}
