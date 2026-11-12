/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;

use crate::kernel::lib::btree::BinaryTree;

#[derive(Debug)]
pub(crate) struct Timer {
    deadline_ns: u64,
}

pub(crate) struct TimerQueue<const N: usize> {
    deadlines: BinaryTree<(u64, u64), Arc<Timer>>,
}

impl<const N: usize> TimerQueue<N> {
    pub(crate) const fn new() -> Self {
        assert!(N > 0);
        Self {
            deadlines: BinaryTree::new(),
        }
    }

    fn timer_address(timer: &Arc<Timer>) -> u64 {
        Arc::as_ptr(timer) as usize as u64
    }

    pub(crate) fn schedule(&mut self, deadline_ns: u64) -> Option<Arc<Timer>> {
        if self.deadlines.len() >= N {
            return None;
        }
        let timer = Arc::new(Timer { deadline_ns });
        let address = Self::timer_address(&timer);
        let previous = self
            .deadlines
            .insert((deadline_ns, address), Arc::clone(&timer));
        debug_assert!(previous.is_none());
        Some(timer)
    }

    pub(crate) fn timer_by_address(&self, address: u64) -> Option<Arc<Timer>> {
        let key = self
            .deadlines
            .find_key_by(|(_, timer_address), timer| {
                *timer_address == address
                    && Self::timer_address(timer) == address
            })?;
        self.deadlines.get(key).cloned()
    }

    pub(crate) fn cancel(&mut self, timer: &Arc<Timer>) -> bool {
        let address = Self::timer_address(timer);
        let Some(key) = self
            .deadlines
            .find_key_by(|(deadline, timer_address), stored_timer| {
                *deadline == timer.deadline_ns
                    && *timer_address == address
                    && Arc::ptr_eq(stored_timer, timer)
            })
            .copied()
        else {
            return false;
        };
        let Some(timer) = self.deadlines.remove(&key) else {
            return false;
        };
        debug_assert_eq!(timer.deadline_ns, key.0);
        drop(timer);
        true
    }

    pub(crate) fn pop_expired(&mut self, now_ns: u64) -> Option<Arc<Timer>> {
        let ((deadline_ns, _), _) = self.deadlines.first()?;
        if *deadline_ns > now_ns {
            return None;
        }

        let ((deadline_ns, address), timer) = self.deadlines.take_min()?;
        debug_assert_eq!(timer.deadline_ns, deadline_ns);
        debug_assert_eq!(Self::timer_address(&timer), address);
        Some(timer)
    }

    pub(crate) fn next_deadline(&self) -> Option<u64> {
        self.deadlines.first().map(|((deadline_ns, _), _)| *deadline_ns)
    }
}

#[cfg(test)]
#[path = "../../tests/kernel/event/timers.rs"]
mod tests;
