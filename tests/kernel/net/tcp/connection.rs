/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
    use alloc::vec::Vec;

    use crate::kernel::arch::traits::CpuArchitecture as _;
    use super::{
        TcpFlowKey, TcpSocket, TcpSocketHandle, TcpSocketTable, TcpState, TcpTransmission,
        MAX_TCP_SEND_BUFFER_PER_SOCKET, MAX_TCP_SEND_BUFFER_TOTAL, MAX_TCP_SOCKETS,
        MAX_TCP_RECEIVE_WINDOW,
    };
    use crate::kernel::net::tcp::segment::{
        write_segment, TcpSegment, TCP_FLAG_ACK, TCP_FLAG_FIN, TCP_FLAG_PSH, TCP_FLAG_RST,
        TCP_FLAG_SYN, TCP_HEADER_LENGTH,
    };
    use crate::kernel::net::wire::Ipv4Address;

    fn encode_transmission(transmission: &TcpTransmission) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.resize(TCP_HEADER_LENGTH + transmission.payload.len(), 0);
        let length = write_segment(
            &mut bytes,
            transmission.source_address,
            transmission.destination_address,
            transmission.source_port,
            transmission.destination_port,
            transmission.sequence_number,
            transmission.acknowledgment_number,
            transmission.flags,
            transmission.window,
            &transmission.payload,
        )
        .unwrap();
        bytes.truncate(length);
        bytes
    }

    #[test]
    fn client_enters_established_after_syn_ack() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 0x1020_3040)
            .unwrap();
        let mut transmissions = Vec::new();
        sockets.poll(1, &mut transmissions).unwrap();
        assert_eq!(transmissions.len(), 1);
        assert_eq!(transmissions[0].flags, TCP_FLAG_SYN);
        let initial_sequence = transmissions[0].sequence_number;
        transmissions.clear();

        let mut response = [0; 20];
        let response_length = write_segment(
            &mut response,
            remote,
            local,
            443,
            49152,
            1234,
            initial_sequence.wrapping_add(1),
            TCP_FLAG_SYN | TCP_FLAG_ACK,
            4096,
            &[],
        )
        .unwrap();
        let response = TcpSegment::parse(&response[..response_length], remote, local).unwrap();
        assert!(sockets
            .receive_segment(remote, local, &response, 2, &mut transmissions)
            .unwrap());
        assert_eq!(sockets.state(handle), Some(TcpState::Established));
        assert_eq!(transmissions.len(), 1);
        assert_eq!(transmissions[0].flags, TCP_FLAG_ACK);
    }

    #[test]
    fn passive_final_ack_payload_is_delivered() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let listener = sockets.listen(local, 8080, 1, 0).unwrap();
        let (index, generation) = sockets.allocate_slot().unwrap();
        let mut child = TcpSocket::new(
            local,
            remote,
            8080,
            49152,
            100,
            TcpState::SynReceived,
            sockets.maximum_segment_size,
        )
        .unwrap();
        child.parent_listener = Some(listener);
        child.receive_next = Some(501);
        let mut transmissions = Vec::new();
        child
            .start_outstanding(TCP_FLAG_SYN | TCP_FLAG_ACK, &[], 0, &mut transmissions)
            .unwrap();
        sockets.slots[index].socket = Some(child);
        let child_handle = TcpSocketHandle { index, generation };
        sockets
            .add_flow(
                TcpFlowKey {
                    local_address: local,
                    remote_address: remote,
                    local_port: 8080,
                    remote_port: 49152,
                },
                child_handle,
            )
            .unwrap();
        transmissions.clear();

        let mut bytes = [0; TCP_HEADER_LENGTH + 4];
        let initial_peer_window = sockets.socket(child_handle).unwrap().peer_window;
        let length = write_segment(
            &mut bytes,
            remote,
            local,
            49152,
            8080,
            501,
            101,
            TCP_FLAG_SYN | TCP_FLAG_ACK,
            1,
            &[],
        )
        .unwrap();
        let segment = TcpSegment::parse(&bytes[..length], remote, local).unwrap();
        assert!(!sockets
            .receive_segment(remote, local, &segment, 1, &mut transmissions)
            .unwrap());
        assert_eq!(sockets.state(child_handle), Some(TcpState::SynReceived));
        assert_eq!(
            sockets.socket(child_handle).unwrap().peer_window,
            initial_peer_window
        );
        assert_eq!(sockets.socket(child_handle).unwrap().outstanding.len(), 1);
        assert_eq!(transmissions.len(), 1);
        assert_eq!(transmissions[0].flags, TCP_FLAG_SYN | TCP_FLAG_ACK);
        transmissions.clear();

        let length = write_segment(
            &mut bytes,
            remote,
            local,
            49152,
            8080,
            501,
            101,
            TCP_FLAG_ACK | TCP_FLAG_PSH,
            4096,
            b"data",
        )
        .unwrap();
        let segment = TcpSegment::parse(&bytes[..length], remote, local).unwrap();
        assert!(sockets
            .receive_segment(remote, local, &segment, 1, &mut transmissions)
            .unwrap());
        let accepted = sockets.accept(listener).unwrap().unwrap();
        let mut output = [0; 4];
        assert_eq!(sockets.receive(accepted, &mut output).unwrap(), 4);
        assert_eq!(&output, b"data");

        transmissions.clear();
        let peer_window = sockets.socket(accepted).unwrap().peer_window;
        let length = write_segment(
            &mut bytes,
            remote,
            local,
            49152,
            8080,
            505,
            0,
            TCP_FLAG_SYN,
            1,
            &[],
        )
        .unwrap();
        let segment = TcpSegment::parse(&bytes[..length], remote, local).unwrap();
        assert!(!sockets
            .receive_segment(remote, local, &segment, 2, &mut transmissions)
            .unwrap());
        assert_eq!(sockets.state(accepted), Some(TcpState::Established));
        assert_eq!(sockets.socket(accepted).unwrap().peer_window, peer_window);
        assert_eq!(transmissions.len(), 1);
        assert_eq!(transmissions[0].flags, TCP_FLAG_ACK);
        assert_eq!(transmissions[0].acknowledgment_number, 505);
    }

    #[test]
    fn simultaneous_close_waits_for_local_fin_ack() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 100)
            .unwrap();
        let mut transmissions = Vec::new();
        {
            let socket = sockets.socket_mut(handle).unwrap();
            socket.state = TcpState::FinWait1;
            socket.receive_next = Some(500);
            socket
                .start_outstanding(TCP_FLAG_ACK | TCP_FLAG_FIN, &[], 0, &mut transmissions)
                .unwrap();
        }
        transmissions.clear();

        let mut bytes = [0; TCP_HEADER_LENGTH];
        let length = write_segment(
            &mut bytes,
            remote,
            local,
            443,
            49152,
            500,
            100,
            TCP_FLAG_ACK | TCP_FLAG_FIN,
            4096,
            &[],
        )
        .unwrap();
        let segment = TcpSegment::parse(&bytes[..length], remote, local).unwrap();
        assert!(sockets
            .receive_segment(remote, local, &segment, 1, &mut transmissions)
            .unwrap());
        assert_eq!(sockets.state(handle), Some(TcpState::Closing));
        assert_eq!(sockets.socket(handle).unwrap().outstanding.len(), 1);

        let length = write_segment(
            &mut bytes,
            remote,
            local,
            443,
            49152,
            501,
            101,
            TCP_FLAG_ACK,
            4096,
            &[],
        )
        .unwrap();
        let segment = TcpSegment::parse(&bytes[..length], remote, local).unwrap();
        assert!(sockets
            .receive_segment(remote, local, &segment, 2, &mut transmissions)
            .unwrap());
        assert_eq!(sockets.state(handle), Some(TcpState::TimeWait));
        assert!(sockets.socket(handle).unwrap().outstanding.is_empty());
        let flow_key = TcpFlowKey {
            local_address: local,
            remote_address: remote,
            local_port: 49152,
            remote_port: 443,
        };
        assert_eq!(sockets.flow_handle(flow_key), Some(handle));

        let length = write_segment(
            &mut bytes,
            remote,
            local,
            443,
            49152,
            500,
            101,
            TCP_FLAG_ACK | TCP_FLAG_FIN,
            4096,
            &[],
        )
        .unwrap();
        let segment = TcpSegment::parse(&bytes[..length], remote, local).unwrap();
        transmissions.clear();
        assert!(!sockets
            .receive_segment(remote, local, &segment, 10, &mut transmissions)
            .unwrap());
        assert_eq!(transmissions.len(), 1);
        assert_eq!(transmissions[0].flags, TCP_FLAG_ACK);
        assert_eq!(
            sockets.socket(handle).unwrap().close_deadline_ns,
            Some(60_000_000_010)
        );

        sockets.poll(60_000_000_009, &mut transmissions).unwrap();
        assert_eq!(sockets.state(handle), Some(TcpState::TimeWait));
        sockets.poll(60_000_000_010, &mut transmissions).unwrap();
        assert_eq!(sockets.state(handle), Some(TcpState::Closed));
        assert_eq!(sockets.flow_handle(flow_key), None);
    }

    #[test]
    fn syn_sent_rejects_rst_with_invalid_acknowledgment() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let initial_sequence = 0x1020_3040;
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, initial_sequence)
            .unwrap();
        let mut bytes = [0; TCP_HEADER_LENGTH];
        let length = write_segment(
            &mut bytes,
            remote,
            local,
            443,
            49152,
            1234,
            initial_sequence.wrapping_add(2),
            TCP_FLAG_RST | TCP_FLAG_ACK,
            4096,
            &[],
        )
        .unwrap();
        let segment = TcpSegment::parse(&bytes[..length], remote, local).unwrap();
        let mut transmissions = Vec::new();

        assert!(!sockets
            .receive_segment(remote, local, &segment, 1, &mut transmissions)
            .unwrap());
        assert_eq!(sockets.state(handle), Some(TcpState::SynSent));
    }

    #[test]
    fn duplicate_live_flow_tuple_is_rejected() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 100)
            .unwrap();
        assert!(matches!(
            sockets.connect_with_initial_sequence(local, remote, 49152, 443, 200),
            Err(crate::kernel::error::Error::AddrInUse)
        ));
        assert_eq!(sockets.slots.iter().filter(|slot| slot.socket.is_some()).count(), 1);
    }

    #[test]
    fn stale_socket_handle_is_rejected() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 0x1020_3040)
            .unwrap();
        assert!(sockets.remove(handle));
        assert_eq!(sockets.state(handle), None);
    }

    #[test]
    fn reset_unaccepted_child_is_reaped_and_skipped_by_accept() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let listener = sockets.listen(local, 8080, 1, 0).unwrap();
        let (index, generation) = sockets.allocate_slot().unwrap();
        let mut child = TcpSocket::new(
            local,
            remote,
            8080,
            49152,
            100,
            TcpState::Established,
            sockets.maximum_segment_size,
        )
        .unwrap();
        child.parent_listener = Some(listener);
        child.receive_next = Some(101);
        sockets.slots[index].socket = Some(child);
        let child_handle = TcpSocketHandle { index, generation };
        sockets
            .add_flow(
                TcpFlowKey {
                    local_address: local,
                    remote_address: remote,
                    local_port: 8080,
                    remote_port: 49152,
                },
                child_handle,
            )
            .unwrap();
        sockets.enqueue_accepted(listener, child_handle).unwrap();

        let mut bytes = [0; TCP_HEADER_LENGTH];
        let length = write_segment(
            &mut bytes,
            remote,
            local,
            49152,
            8080,
            101,
            0,
            TCP_FLAG_RST,
            4096,
            &[],
        )
        .unwrap();
        let segment = TcpSegment::parse(&bytes[..length], remote, local).unwrap();
        let mut transmissions = Vec::new();
        assert!(sockets
            .receive_segment(remote, local, &segment, 10, &mut transmissions)
            .unwrap());
        sockets.poll(10, &mut transmissions).unwrap();

        assert_eq!(sockets.state(child_handle), None);
        assert!(!sockets.readable(listener));
        let (next_index, next_generation) = sockets.allocate_slot().unwrap();
        let mut next_child = TcpSocket::new(
            local,
            remote,
            8080,
            49153,
            200,
            TcpState::Established,
            sockets.maximum_segment_size,
        )
        .unwrap();
        next_child.parent_listener = Some(listener);
        sockets.slots[next_index].socket = Some(next_child);
        let next_handle = TcpSocketHandle {
            index: next_index,
            generation: next_generation,
        };
        sockets
            .add_flow(
                TcpFlowKey {
                    local_address: local,
                    remote_address: remote,
                    local_port: 8080,
                    remote_port: 49153,
                },
                next_handle,
            )
            .unwrap();
        sockets.enqueue_accepted(listener, next_handle).unwrap();
        assert_eq!(sockets.accept(listener).unwrap(), Some(next_handle));
    }

    #[test]
    fn out_of_window_segment_and_future_ack_do_not_mutate_socket_state() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 100)
            .unwrap();
        let mut initial_transmissions = Vec::new();
        {
            let socket = sockets.socket_mut(handle).unwrap();
            socket.state = TcpState::Established;
            socket.receive_next = Some(1000);
            socket.peer_window = 4096;
            socket
                .start_outstanding(
                    TCP_FLAG_ACK | TCP_FLAG_PSH,
                    b"x",
                    0,
                    &mut initial_transmissions,
                )
                .unwrap();
        }
        let acknowledgment_number = sockets.socket(handle).unwrap().send_next;
        let mut bytes = [0; TCP_HEADER_LENGTH + 1];
        let length = write_segment(
            &mut bytes,
            remote,
            local,
            443,
            49152,
            1000,
            acknowledgment_number,
            TCP_FLAG_PSH,
            8192,
            b"x",
        )
        .unwrap();
        let segment = TcpSegment::parse(&bytes[..length], remote, local).unwrap();
        let mut transmissions = Vec::new();
        assert!(!sockets
            .receive_segment(remote, local, &segment, 1, &mut transmissions)
            .unwrap());
        assert_eq!(sockets.socket(handle).unwrap().peer_window, 4096);
        assert!(sockets.socket(handle).unwrap().receive_buffer.is_empty());

        let mut bytes = [0; TCP_HEADER_LENGTH];
        let length = write_segment(
            &mut bytes,
            remote,
            local,
            443,
            49152,
            1000 + MAX_TCP_RECEIVE_WINDOW as u32,
            acknowledgment_number,
            TCP_FLAG_ACK,
            8192,
            &[],
        )
        .unwrap();
        let segment = TcpSegment::parse(&bytes[..length], remote, local).unwrap();

        assert!(!sockets
            .receive_segment(remote, local, &segment, 1, &mut transmissions)
            .unwrap());
        let socket = sockets.socket(handle).unwrap();
        assert_eq!(socket.peer_window, 4096);
        assert_eq!(socket.outstanding.len(), 1);

        transmissions.clear();
        let length = write_segment(
            &mut bytes,
            remote,
            local,
            443,
            49152,
            1000,
            acknowledgment_number.wrapping_add(1),
            TCP_FLAG_ACK,
            8192,
            &[],
        )
        .unwrap();
        let segment = TcpSegment::parse(&bytes[..length], remote, local).unwrap();
        assert!(!sockets
            .receive_segment(remote, local, &segment, 2, &mut transmissions)
            .unwrap());
        let socket = sockets.socket(handle).unwrap();
        assert_eq!(socket.peer_window, 4096);
        assert_eq!(socket.outstanding.len(), 1);
        assert_eq!(socket.send_unacknowledged, 100);
        assert_eq!(transmissions.len(), 1);
        assert_eq!(transmissions[0].flags, TCP_FLAG_ACK);
        assert_eq!(transmissions[0].sequence_number, acknowledgment_number);
        assert_eq!(transmissions[0].acknowledgment_number, 1000);
    }

    #[test]
    fn socket_table_grows_beyond_initial_capacity() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        for index in 0..MAX_TCP_SOCKETS {
            sockets
                .connect_with_initial_sequence(
                    local,
                    remote,
                    49152 + index as u16,
                    443,
                    index as u32,
                )
                .unwrap();
        }
        assert_eq!(sockets.slots.len(), MAX_TCP_SOCKETS);
        assert!(matches!(
            sockets.connect_with_initial_sequence(local, remote, 49408, 443, 1),
            Err(crate::kernel::error::Error::NoBufs)
        ));
    }

    #[test]
    fn send_buffer_limits_accept_partial_data_then_apply_backpressure() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 0x1020_3040)
            .unwrap();
        sockets.socket_mut(handle).unwrap().state = TcpState::Established;
        let mut payload = Vec::new();
        payload.resize(MAX_TCP_SEND_BUFFER_PER_SOCKET + 1, 0x5a);

        assert_eq!(
            sockets.send(handle, &payload).unwrap(),
            MAX_TCP_SEND_BUFFER_PER_SOCKET
        );
        assert_eq!(sockets.queued_send_bytes, MAX_TCP_SEND_BUFFER_PER_SOCKET);
        assert!(matches!(
            sockets.send(handle, b"next"),
            Err(crate::kernel::error::Error::NoBufs)
        ));
        assert!(!sockets.writable(handle));
        assert!(sockets.remove(handle));
        assert_eq!(sockets.queued_send_bytes, 0);

        for index in 0..(MAX_TCP_SEND_BUFFER_TOTAL / MAX_TCP_SEND_BUFFER_PER_SOCKET) {
            let handle = sockets
                .connect_with_initial_sequence(
                    local,
                    remote,
                    49152 + index as u16,
                    443,
                    index as u32,
                )
                .unwrap();
            sockets.socket_mut(handle).unwrap().state = TcpState::Established;
            assert_eq!(
                sockets
                    .send(handle, &payload[..MAX_TCP_SEND_BUFFER_PER_SOCKET])
                    .unwrap(),
                MAX_TCP_SEND_BUFFER_PER_SOCKET
            );
        }
        assert_eq!(sockets.queued_send_bytes, MAX_TCP_SEND_BUFFER_TOTAL);
        let overflow_handle = sockets
            .connect_with_initial_sequence(local, remote, 49216, 443, 1)
            .unwrap();
        sockets.socket_mut(overflow_handle).unwrap().state = TcpState::Established;
        assert!(matches!(
            sockets.send(overflow_handle, b"x"),
            Err(crate::kernel::error::Error::NoBufs)
        ));
        assert!(!sockets.writable(overflow_handle));
    }

    #[test]
    fn out_of_order_payload_is_reassembled_and_fin_waits_for_gap() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 0x1020_3040)
            .unwrap();
        let socket = sockets.socket_mut(handle).unwrap();
        socket.state = TcpState::Established;
        socket.receive_next = Some(1000);

        let future_segment = TcpSegment {
            source_port: 443,
            destination_port: 49152,
            sequence_number: 1005,
            acknowledgment_number: 0,
            flags: TCP_FLAG_ACK | crate::kernel::net::tcp::segment::TCP_FLAG_FIN,
            window: 4096,
            payload: b"world",
        };
        let mut transmissions = Vec::new();
        assert!(sockets
            .receive_segment(remote, local, &future_segment, 2, &mut transmissions)
            .unwrap());
        assert_eq!(transmissions[0].acknowledgment_number, 1000);
        assert_eq!(
            transmissions[0].window,
            (MAX_TCP_RECEIVE_WINDOW - b"world".len()) as u16
        );
        assert_eq!(
            sockets.socket(handle).unwrap().out_of_order_bytes,
            b"world".len()
        );

        let overlapping_segment = TcpSegment {
            source_port: 443,
            destination_port: 49152,
            sequence_number: 1007,
            acknowledgment_number: 0,
            flags: TCP_FLAG_ACK,
            window: 4096,
            payload: b"rl",
        };
        transmissions.clear();
        assert!(!sockets
            .receive_segment(remote, local, &overlapping_segment, 3, &mut transmissions)
            .unwrap());

        let in_order_segment = TcpSegment {
            source_port: 443,
            destination_port: 49152,
            sequence_number: 1000,
            acknowledgment_number: 0,
            flags: TCP_FLAG_ACK,
            window: 4096,
            payload: b"hello",
        };
        transmissions.clear();
        assert!(sockets
            .receive_segment(remote, local, &in_order_segment, 4, &mut transmissions)
            .unwrap());
        assert_eq!(sockets.state(handle), Some(TcpState::CloseWait));
        assert_eq!(transmissions[0].acknowledgment_number, 1011);

        let mut output = [0; 10];
        assert_eq!(sockets.receive(handle, &mut output).unwrap(), output.len());
        assert_eq!(&output, b"helloworld");
        assert_eq!(sockets.socket(handle).unwrap().out_of_order_bytes, 0);
    }

    #[test]
    fn congestion_window_grows_on_ack_and_reduces_on_timeout() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 0x1020_3040)
            .unwrap();
        let socket = sockets.socket_mut(handle).unwrap();
        socket.state = TcpState::Established;
        let initial_window = socket.congestion_window_bytes;
        let mut transmissions = Vec::new();
        socket
            .start_outstanding(
                TCP_FLAG_ACK | crate::kernel::net::tcp::segment::TCP_FLAG_PSH,
                &[0x5a; 1200],
                1,
                &mut transmissions,
            )
            .unwrap();
        let acknowledgment_number = socket.send_next;
        assert!(socket.acknowledge_outstanding(acknowledgment_number, 2));
        assert_eq!(socket.congestion_window_bytes, initial_window + 1200);

        let previous_window = socket.congestion_window_bytes;
        socket
            .start_outstanding(
                TCP_FLAG_ACK | crate::kernel::net::tcp::segment::TCP_FLAG_PSH,
                &[0xa5; 1000],
                2,
                &mut transmissions,
            )
            .unwrap();
        socket.on_retransmission_timeout();
        assert_eq!(socket.congestion_window_bytes, socket.maximum_segment_size);
        assert_eq!(
            socket.slow_start_threshold_bytes,
            (previous_window / 2).max(socket.maximum_segment_size * 2)
        );
    }

    #[test]
    fn poll_fills_but_does_not_exceed_congestion_window() {
        let local = Ipv4Address([192, 0, 2, 1]);
        let remote = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let handle = sockets
            .connect_with_initial_sequence(local, remote, 49152, 443, 0x1020_3040)
            .unwrap();
        let maximum_segment_size = sockets.maximum_segment_size;
        let socket = sockets.socket_mut(handle).unwrap();
        socket.state = TcpState::Established;
        socket.receive_next = Some(0);
        socket.congestion_window_bytes = maximum_segment_size * 3;
        let mut payload = Vec::new();
        payload.resize(maximum_segment_size * 4, 0x3c);
        assert_eq!(sockets.send(handle, &payload).unwrap(), payload.len());

        let mut transmissions = Vec::new();
        sockets.poll(10, &mut transmissions).unwrap();
        assert_eq!(transmissions.len(), 3);
        let socket = sockets.socket(handle).unwrap();
        assert_eq!(socket.outstanding.len(), 3);
        assert_eq!(socket.outstanding_payload_bytes, maximum_segment_size * 3);
        assert_eq!(socket.send_buffer.len(), maximum_segment_size);

        let initial_sequence = 0x1020_3040;
        let partial_ack_number = initial_sequence + (maximum_segment_size * 3 / 2) as u32;
        let partial_ack = TcpSegment {
            source_port: 443,
            destination_port: 49152,
            sequence_number: 0,
            acknowledgment_number: partial_ack_number,
            flags: TCP_FLAG_ACK,
            window: u16::MAX,
            payload: &[],
        };
        assert!(sockets
            .receive_segment(remote, local, &partial_ack, 11, &mut Vec::new())
            .unwrap());
        let socket = sockets.socket(handle).unwrap();
        assert_eq!(socket.outstanding.len(), 2);
        assert_eq!(
            socket.outstanding_payload_bytes,
            maximum_segment_size * 3 - maximum_segment_size * 3 / 2
        );

        let cumulative_ack = TcpSegment {
            acknowledgment_number: initial_sequence
                .wrapping_add((maximum_segment_size * 3) as u32),
            ..partial_ack
        };
        assert!(sockets
            .receive_segment(remote, local, &cumulative_ack, 12, &mut Vec::new())
            .unwrap());
        let socket = sockets.socket(handle).unwrap();
        assert!(socket.outstanding.is_empty());
        assert_eq!(socket.outstanding_payload_bytes, 0);
    }

    #[test]
    fn listener_accepts_completed_handshake() {
        if crate::kernel::arch::CpuArchitecture::random_u32().is_none() {
            return;
        }
        let server = Ipv4Address([192, 0, 2, 1]);
        let client = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = TcpSocketTable::new(1500).unwrap();
        let listener = sockets.listen(server, 8080, 4, 1).unwrap();
        let mut transmissions = Vec::new();

        let mut syn_bytes = [0; 20];
        let syn_length = write_segment(
            &mut syn_bytes,
            client,
            server,
            49152,
            8080,
            100,
            0,
            TCP_FLAG_SYN,
            4096,
            &[],
        )
        .unwrap();
        let syn = TcpSegment::parse(&syn_bytes[..syn_length], client, server).unwrap();
        assert!(sockets
            .receive_segment(client, server, &syn, 2, &mut transmissions)
            .unwrap());
        assert_eq!(transmissions.len(), 1);
        assert_eq!(transmissions[0].flags, TCP_FLAG_SYN | TCP_FLAG_ACK);
        let server_sequence = transmissions[0].sequence_number;
        transmissions.clear();

        let mut ack_bytes = [0; 20];
        let ack_length = write_segment(
            &mut ack_bytes,
            client,
            server,
            49152,
            8080,
            101,
            server_sequence.wrapping_add(1),
            TCP_FLAG_ACK,
            4096,
            &[],
        )
        .unwrap();
        let ack = TcpSegment::parse(&ack_bytes[..ack_length], client, server).unwrap();
        assert!(sockets
            .receive_segment(client, server, &ack, 3, &mut transmissions)
            .unwrap());
        let accepted = sockets.accept(listener).unwrap().unwrap();
        assert_eq!(sockets.state(accepted), Some(TcpState::Established));
    }

    #[test]
    fn two_tcp_endpoints_exchange_payload_and_acknowledge_it() {
        if crate::kernel::arch::CpuArchitecture::random_u32().is_none() {
            return;
        }
        let client_address = Ipv4Address([192, 0, 2, 1]);
        let server_address = Ipv4Address([192, 0, 2, 2]);
        let mut client = TcpSocketTable::new(1500).unwrap();
        let mut server = TcpSocketTable::new(1500).unwrap();
        let listener = server.listen(server_address, 8080, 4, 1).unwrap();
        let client_socket = client
            .connect_with_initial_sequence(client_address, server_address, 49152, 8080, 1000)
            .unwrap();
        let mut client_transmissions = Vec::new();
        let mut server_transmissions = Vec::new();

        client.poll(1, &mut client_transmissions).unwrap();
        let syn_bytes = encode_transmission(&client_transmissions.remove(0));
        let syn = TcpSegment::parse(&syn_bytes, client_address, server_address).unwrap();
        assert!(server
            .receive_segment(client_address, server_address, &syn, 2, &mut server_transmissions)
            .unwrap());

        let syn_ack_bytes = encode_transmission(&server_transmissions.remove(0));
        let syn_ack = TcpSegment::parse(&syn_ack_bytes, server_address, client_address).unwrap();
        assert!(client
            .receive_segment(server_address, client_address, &syn_ack, 3, &mut client_transmissions)
            .unwrap());

        let ack_bytes = encode_transmission(&client_transmissions.remove(0));
        let ack = TcpSegment::parse(&ack_bytes, client_address, server_address).unwrap();
        assert!(server
            .receive_segment(client_address, server_address, &ack, 4, &mut server_transmissions)
            .unwrap());
        let accepted = server.accept(listener).unwrap().unwrap();
        assert_eq!(client.state(client_socket), Some(TcpState::Established));
        assert_eq!(server.state(accepted), Some(TcpState::Established));

        assert_eq!(client.send(client_socket, b"payload").unwrap(), 7);
        client.poll(5, &mut client_transmissions).unwrap();
        let data_bytes = encode_transmission(&client_transmissions.remove(0));
        let data = TcpSegment::parse(&data_bytes, client_address, server_address).unwrap();
        assert_eq!(data.flags & TCP_FLAG_PSH, TCP_FLAG_PSH);
        assert!(server
            .receive_segment(client_address, server_address, &data, 6, &mut server_transmissions)
            .unwrap());
        let mut output = [0; 7];
        assert_eq!(server.receive(accepted, &mut output).unwrap(), output.len());
        assert_eq!(&output, b"payload");

        let data_ack_bytes = encode_transmission(&server_transmissions.remove(0));
        let data_ack = TcpSegment::parse(&data_ack_bytes, server_address, client_address).unwrap();
        assert!(client
            .receive_segment(server_address, client_address, &data_ack, 7, &mut client_transmissions)
            .unwrap());
        assert!(client.writable(client_socket));
    }
