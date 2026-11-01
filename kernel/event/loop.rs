use crate::event_completion::{Completion, CompletionQueue};
use crate::event_timers::{TimerId, TimerQueue};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Event {
    Timer(TimerId),
    Completion(Completion),
}

pub(crate) struct EventLoop<const N: usize> {
    timers: TimerQueue<N>,
    events: CompletionQueue<Event, N>,
}

impl<const N: usize> EventLoop<N> {
    pub(crate) const fn new() -> Self {
        Self {
            timers: TimerQueue::new(),
            events: CompletionQueue::new(),
        }
    }

    pub(crate) fn schedule_timer(&mut self, deadline_ns: u64) -> Option<TimerId> {
        self.timers.schedule(deadline_ns)
    }

    pub(crate) fn cancel_timer(&mut self, timer: TimerId) -> bool {
        self.timers.cancel(timer)
    }

    pub(crate) fn complete(&mut self, completion: Completion) -> Result<(), Completion> {
        self.events
            .push(Event::Completion(completion))
            .map_err(|event| match event {
                Event::Completion(completion) => completion,
                Event::Timer(_) => unreachable!(),
            })
    }

    pub(crate) fn poll(&mut self, now_ns: u64) -> usize {
        let mut processed = 0;
        while self.events.len() < N {
            let Some(timer) = self.timers.pop_expired(now_ns) else {
                break;
            };
            if self.events.push(Event::Timer(timer)).is_err() {
                break;
            }
            processed += 1;
        }
        processed
    }

    pub(crate) fn next_deadline(&self) -> Option<u64> {
        self.timers.next_deadline()
    }

    pub(crate) fn next_event(&mut self) -> Option<Event> {
        self.events.pop()
    }
}
