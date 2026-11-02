/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

const NO_INDEX: usize = usize::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TimerId {
    index: usize,
    generation: u64,
}

#[derive(Clone, Copy)]
struct Timer {
    id: TimerId,
    deadline_ns: u64,
    heap_index: usize,
    next_free: usize,
}

impl Timer {
    const EMPTY: Self = Self {
        id: TimerId {
            index: NO_INDEX,
            generation: 0,
        },
        deadline_ns: 0,
        heap_index: NO_INDEX,
        next_free: NO_INDEX,
    };
}

pub(crate) struct TimerQueue<const N: usize> {
    timers: [Timer; N],
    heap: [usize; N],
    heap_len: usize,
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
            heap: [0; N],
            heap_len: 0,
            free_head: 0,
        }
    }

    pub(crate) fn schedule(&mut self, deadline_ns: u64) -> Option<TimerId> {
        let index = self.free_head;
        if index == NO_INDEX {
            return None;
        }
        self.free_head = self.timers[index].next_free;

        let generation = self.timers[index].id.generation.wrapping_add(1).max(1);
        let id = TimerId { index, generation };
        self.timers[index] = Timer {
            id,
            deadline_ns,
            heap_index: self.heap_len,
            next_free: NO_INDEX,
        };
        self.heap[self.heap_len] = index;
        self.heap_len += 1;
        self.sift_up(self.heap_len - 1);
        Some(id)
    }

    pub(crate) fn cancel(&mut self, id: TimerId) -> bool {
        let Some(timer) = self.timers.get(id.index) else {
            return false;
        };
        if timer.id != id || timer.heap_index == NO_INDEX {
            return false;
        }

        let heap_index = timer.heap_index;
        self.remove_heap_entry(heap_index);
        self.release(id.index);
        true
    }

    pub(crate) fn pop_expired(&mut self, now_ns: u64) -> Option<TimerId> {
        if self.heap_len == 0 {
            return None;
        }
        let index = self.heap[0];
        if self.timers[index].deadline_ns > now_ns {
            return None;
        }

        let id = self.timers[index].id;
        self.remove_heap_entry(0);
        self.release(index);
        Some(id)
    }

    pub(crate) fn next_deadline(&self) -> Option<u64> {
        (self.heap_len > 0).then(|| self.timers[self.heap[0]].deadline_ns)
    }

    fn release(&mut self, index: usize) {
        self.timers[index].heap_index = NO_INDEX;
        self.timers[index].next_free = self.free_head;
        self.free_head = index;
    }

    fn remove_heap_entry(&mut self, index: usize) {
        let removed_slot = self.heap[index];
        let last_index = self.heap_len - 1;
        if index != last_index {
            let replacement = self.heap[last_index];
            self.heap[index] = replacement;
            self.timers[replacement].heap_index = index;
        }
        self.heap_len = last_index;
        self.timers[removed_slot].heap_index = NO_INDEX;

        if index < self.heap_len {
            let parent = index.saturating_sub(1) / 2;
            if index > 0 && self.precedes(self.heap[index], self.heap[parent]) {
                self.sift_up(index);
            } else {
                self.sift_down(index);
            }
        }
    }

    fn sift_up(&mut self, mut index: usize) {
        while index > 0 {
            let parent = (index - 1) / 2;
            let child_slot = self.heap[index];
            let parent_slot = self.heap[parent];
            if !self.precedes(child_slot, parent_slot) {
                break;
            }
            self.heap.swap(index, parent);
            self.timers[child_slot].heap_index = parent;
            self.timers[parent_slot].heap_index = index;
            index = parent;
        }
    }

    fn sift_down(&mut self, mut index: usize) {
        loop {
            let left = index * 2 + 1;
            if left >= self.heap_len {
                break;
            }
            let right = left + 1;
            let child = if right < self.heap_len
                && self.precedes(self.heap[right], self.heap[left])
            {
                right
            } else {
                left
            };
            if !self.precedes(self.heap[child], self.heap[index]) {
                break;
            }

            let child_slot = self.heap[child];
            let parent_slot = self.heap[index];
            self.heap.swap(index, child);
            self.timers[child_slot].heap_index = index;
            self.timers[parent_slot].heap_index = child;
            index = child;
        }
    }

    fn precedes(&self, left: usize, right: usize) -> bool {
        let left = &self.timers[left];
        let right = &self.timers[right];
        left.deadline_ns < right.deadline_ns
            || (left.deadline_ns == right.deadline_ns && left.id.index < right.id.index)
    }
}

#[cfg(test)]
mod tests {
    use super::TimerQueue;

    #[test]
    fn expires_timers_in_deadline_order() {
        let mut queue = TimerQueue::<3>::new();
        let late = queue.schedule(30).unwrap();
        let first = queue.schedule(10).unwrap();
        let middle = queue.schedule(20).unwrap();

        assert_eq!(queue.schedule(40), None);
        assert_eq!(queue.next_deadline(), Some(10));
        assert_eq!(queue.pop_expired(9), None);
        assert_eq!(queue.pop_expired(10), Some(first));
        assert_eq!(queue.pop_expired(20), Some(middle));
        assert_eq!(queue.pop_expired(29), None);
        assert_eq!(queue.pop_expired(30), Some(late));
        assert_eq!(queue.next_deadline(), None);
    }

    #[test]
    fn cancellation_reuses_slots_without_accepting_stale_ids() {
        let mut queue = TimerQueue::<2>::new();
        let cancelled = queue.schedule(10).unwrap();
        let retained = queue.schedule(20).unwrap();

        assert!(queue.cancel(cancelled));
        let replacement = queue.schedule(30).unwrap();
        assert_ne!(replacement, cancelled);
        assert!(!queue.cancel(cancelled));
        assert!(queue.cancel(retained));
        assert_eq!(queue.next_deadline(), Some(30));
        assert_eq!(queue.pop_expired(30), Some(replacement));
    }

    #[test]
    fn cancelling_heap_root_preserves_order() {
        let mut queue = TimerQueue::<4>::new();
        let first = queue.schedule(5).unwrap();
        let later = queue.schedule(15).unwrap();
        let middle = queue.schedule(10).unwrap();

        assert!(queue.cancel(first));
        assert_eq!(queue.next_deadline(), Some(10));
        assert_eq!(queue.pop_expired(10), Some(middle));
        assert_eq!(queue.pop_expired(15), Some(later));
    }
}
