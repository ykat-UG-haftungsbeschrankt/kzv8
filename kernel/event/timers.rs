#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TimerId(u64);

#[derive(Clone, Copy)]
struct Timer {
    id: TimerId,
    deadline_ns: u64,
    active: bool,
}

impl Timer {
    const EMPTY: Self = Self {
        id: TimerId(0),
        deadline_ns: 0,
        active: false,
    };
}

pub(crate) struct TimerQueue<const N: usize> {
    timers: [Timer; N],
    next_id: u64,
}

impl<const N: usize> TimerQueue<N> {
    pub(crate) const fn new() -> Self {
        assert!(N > 0);
        Self {
            timers: [Timer::EMPTY; N],
            next_id: 1,
        }
    }

    pub(crate) fn schedule(&mut self, deadline_ns: u64) -> Option<TimerId> {
        let timer = self.timers.iter_mut().find(|timer| !timer.active)?;
        let id = TimerId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        *timer = Timer {
            id,
            deadline_ns,
            active: true,
        };
        Some(id)
    }

    pub(crate) fn cancel(&mut self, id: TimerId) -> bool {
        if let Some(timer) = self
            .timers
            .iter_mut()
            .find(|timer| timer.active && timer.id == id)
        {
            timer.active = false;
            true
        } else {
            false
        }
    }

    pub(crate) fn pop_expired(&mut self, now_ns: u64) -> Option<TimerId> {
        let index = self
            .timers
            .iter()
            .enumerate()
            .filter(|(_, timer)| timer.active && timer.deadline_ns <= now_ns)
            .min_by_key(|(_, timer)| timer.deadline_ns)
            .map(|(index, _)| index)?;
        self.timers[index].active = false;
        Some(self.timers[index].id)
    }

    pub(crate) fn next_deadline(&self) -> Option<u64> {
        self.timers
            .iter()
            .filter(|timer| timer.active)
            .map(|timer| timer.deadline_ns)
            .min()
    }
}
