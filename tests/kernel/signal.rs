/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
    use super::{
        SignalInfo, SignalQueue, SignalSet, SignalState, SIGALRM, SI_TIMER,
        SIGNAL_QUEUE_CAPACITY,
    };

    fn information(signal_number: u8, value: usize) -> SignalInfo {
        SignalInfo::new(signal_number, 0, 1, 2, value, 0)
    }

    #[test]
    fn standard_signals_coalesce_and_realtime_signals_remain_queued() {
        let mut queue = SignalQueue::new();
        queue.push(information(10, 1)).unwrap();
        queue.push(information(10, 2)).unwrap();
        queue.push(information(34, 3)).unwrap();
        queue.push(information(34, 4)).unwrap();

        assert_eq!(queue.pending_set(), SignalSet::from_signal(10).insert(34));
        assert_eq!(queue.take(SignalSet::from_signal(10)).unwrap().value, 1);
        assert_eq!(queue.take(SignalSet::from_signal(34)).unwrap().value, 3);
        assert_eq!(queue.take(SignalSet::from_signal(34)).unwrap().value, 4);
    }

    #[test]
    fn signal_queue_is_bounded_and_selects_lowest_matching_signal() {
        let mut queue = SignalQueue::new();
        for value in 0..SIGNAL_QUEUE_CAPACITY {
            queue.push(information(34, value)).unwrap();
        }
        assert!(queue.push(information(63, 0)).is_err());
        assert_eq!(queue.take(SignalSet::from_signal(34)).unwrap().value, 0);

        let mut ordered = SignalQueue::new();
        ordered.push(information(35, 1)).unwrap();
        ordered.push(information(34, 2)).unwrap();
        ordered.push(information(35, 3)).unwrap();
        let selected = ordered
            .take(SignalSet::from_signal(35).insert(34))
            .unwrap();
        assert_eq!(selected.signal_number, 34);
        assert_eq!(ordered.take(SignalSet::from_signal(35)).unwrap().value, 1);
    }

    #[test]
    fn signal_sets_exclude_kill_and_stop_from_masks() {
        let set = SignalSet::ALL.without_uncatchable();
        assert!(!set.contains(9));
        assert!(!set.contains(19));
        assert!(set.contains(63));
    }

    #[test]
    fn alarm_replacement_reports_remaining_time_and_expires_once() {
        let mut signals = SignalState::new();
        assert_eq!(signals.set_alarm(100, 2), 0);
        assert_eq!(signals.set_alarm(1_000_000_100, 3), 1);
        assert!(!signals.expire_alarm(4_000_000_099));
        assert!(signals.expire_alarm(4_000_000_100));
        assert_eq!(
            signals
                .pending
                .take(SignalSet::from_signal(SIGALRM))
                .unwrap()
                .code,
            SI_TIMER
        );
        assert!(!signals.expire_alarm(4_000_000_100));
        assert_eq!(SIGNAL_QUEUE_CAPACITY, 32);
    }
