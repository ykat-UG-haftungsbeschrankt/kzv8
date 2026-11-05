/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::lib::btree::BinaryTree;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TimerId {
    index: usize,
    generation: u64,
}

impl TimerId {
    const NO_INDEX: usize = usize::MAX;
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

#[derive(Clone, Copy)]
struct Timer {
    id: TimerId,
    deadline_ns: u64,
    active: bool,
    next_free: usize,
}

impl Timer {
    const EMPTY: Self = Self {
        id: TimerId {
            index: TimerId::NO_INDEX,
            generation: 0,
        },
        deadline_ns: 0,
        active: false,
        next_free: TimerId::NO_INDEX,
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
            timers[index].next_free = if index + 1 < N { index + 1 } else { TimerId::NO_INDEX };
            index += 1;
        }
        Self {
            timers,
            deadlines: BinaryTree::new(),
            free_head: 0,
        }
    }

    pub(crate) fn schedule(&mut self, deadline_ns: u64) -> Option<TimerId> {
        let index = self.free_head;
        if index == TimerId::NO_INDEX {
            return None;
        }
        self.free_head = self.timers[index].next_free;

        let generation = self.timers[index].id.generation.wrapping_add(1).max(1);
        let id = TimerId { index, generation };
        self.timers[index] = Timer {
            id,
            deadline_ns,
            active: true,
            next_free: TimerId::NO_INDEX,
        };
        let previous = self.deadlines.insert((deadline_ns, index), index);
        debug_assert!(previous.is_none());
        Some(id)
    }

    pub(crate) fn cancel(&mut self, id: TimerId) -> bool {
        let Some(timer) = self.timers.get(id.index) else {
            return false;
        };
        if timer.id != id || !timer.active {
            return false;
        }

        let deadline_ns = timer.deadline_ns;
        if self.deadlines.remove(&(deadline_ns, id.index)).is_none() {
            return false;
        }
        self.release(id.index);
        true
    }

    pub(crate) fn pop_expired(&mut self, now_ns: u64) -> Option<TimerId> {
        let ((deadline_ns, _), _) = self.deadlines.first()?;
        if *deadline_ns > now_ns {
            return None;
        }

        let ((_, index), _) = self.deadlines.take_min()?;
        let id = self.timers[index].id;
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
