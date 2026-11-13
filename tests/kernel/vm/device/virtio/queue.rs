/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
    use super::GuestVirtQueue;

    #[test]
    fn configured_queue_maximum_exceeds_previous_fixed_limit() {
        let mut queue = GuestVirtQueue::new(256);
        assert_eq!(queue.max_size(), 256);
        assert!(queue.set_size(256));
        assert_eq!(queue.size(), 256);
    }
