/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
    use super::{NetworkSwitch, MAX_FRAME_SIZE, MAX_HOST_QUEUE_BYTES};

    #[test]
    fn peer_list_grows_beyond_the_old_fixed_capacity() {
        let mut switch = NetworkSwitch::new();
        for _ in 0..9 {
            assert!(switch.register().is_some());
        }
        assert_eq!(switch.peers.len(), 9);
    }

    #[test]
    fn host_queue_byte_quota_counts_dropped_frames() {
        let mut switch = NetworkSwitch::new();
        let host_mac = [0x02, 0x4b, 0x5a, 0, 0, 0xff];
        switch.register_host(host_mac);
        let (peer, source_mac) = switch.register().unwrap();
        let mut frame = [0; MAX_FRAME_SIZE];
        frame[..6].copy_from_slice(&host_mac);
        frame[6..12].copy_from_slice(&source_mac);
        frame[12..14].copy_from_slice(&0x0800u16.to_be_bytes());

        for _ in 0..(MAX_HOST_QUEUE_BYTES / MAX_FRAME_SIZE + 2) {
            switch.transmit(peer, &frame);
        }
        assert!(switch.dropped_frame_count() > 0);
        assert!(switch.host_stack_bytes <= MAX_HOST_QUEUE_BYTES);
    }

    #[test]
    fn undersized_host_stack_buffer_drops_head_without_blocking_queue() {
        let mut switch = NetworkSwitch::new();
        let frame = [0x5a; MAX_FRAME_SIZE];
        switch.enqueue_host_stack(&frame);
        switch.enqueue_host_stack(&frame[..32]);
        let mut output = [0; 64];

        assert_eq!(switch.receive_host_stack(&mut output), Some(0));
        assert_eq!(switch.dropped_frame_count(), 1);
        assert_eq!(switch.receive_host_stack(&mut output), Some(32));
    }

    #[test]
    fn undersized_host_transmit_buffer_drops_head_without_blocking_queue() {
        let mut switch = NetworkSwitch::new();
        let frame = [0x5a; MAX_FRAME_SIZE];
        switch.enqueue_host_transmit(&frame);
        switch.enqueue_host_transmit(&frame[..32]);
        let mut output = [0; 64];

        assert_eq!(switch.peek_host_transmit(&mut output), Some(0));
        assert_eq!(switch.dropped_frame_count(), 1);
        assert_eq!(switch.peek_host_transmit(&mut output), Some(32));
    }
