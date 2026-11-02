#[cfg(test)]
mod tests {
    use super::{Event, EventFlags, EventLoop, EventSourceId};
    use crate::kernel::error::Error;
    use crate::kernel::event::completion::Completion;

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
        events.set_readiness(token, EventFlags::READABLE).unwrap();
        events.modify(token, EventFlags::WRITABLE, 2).unwrap();
        assert_eq!(events.wait(0), None);

        events.set_readiness(token, EventFlags::WRITABLE).unwrap();
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
            Err(Error::NoEnt)
        );
        let replacement = events.register(source, EventFlags::READABLE, 3).unwrap();
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
            Err(Error::Exist)
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
