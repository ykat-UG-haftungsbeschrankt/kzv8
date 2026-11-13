/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
    use alloc::vec::Vec;

    use super::{
        write_datagram, UdpDatagram, UdpSocketTable,
        MAX_UDP_RECEIVE_BYTES_PER_SOCKET, MAX_UDP_RECEIVE_DATAGRAMS_PER_SOCKET,
        MAX_UDP_SOCKETS,
    };
    use crate::kernel::net::wire::Ipv4Address;

    #[test]
    fn datagram_round_trip_preserves_ports_payload_and_checksum() {
        let client = Ipv4Address([192, 0, 2, 1]);
        let server = Ipv4Address([192, 0, 2, 2]);
        let payload = b"hello over udp";
        let mut bytes = [0; 64];
        let length = write_datagram(&mut bytes, client, server, 49152, 53, payload).unwrap();
        let datagram = UdpDatagram::parse(&bytes[..length], client, server).unwrap();
        assert_eq!(datagram.source_port, 49152);
        assert_eq!(datagram.destination_port, 53);
        assert_eq!(datagram.payload, payload);
    }

    #[test]
    fn bound_socket_receives_datagram_with_source_endpoint() {
        let client = Ipv4Address([192, 0, 2, 1]);
        let server = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = UdpSocketTable::new().unwrap();
        let socket = sockets.create().unwrap();
        sockets.bind(socket, server, 53).unwrap();
        let mut bytes = [0; 64];
        let length = write_datagram(&mut bytes, client, server, 49152, 53, b"query").unwrap();
        let datagram = UdpDatagram::parse(&bytes[..length], client, server).unwrap();
        assert!(sockets
            .receive_datagram(client, server, &datagram)
            .unwrap());

        let mut output = Vec::new();
        output.resize(16, 0);
        let received = sockets.receive(socket, &mut output).unwrap().unwrap();
        assert_eq!(received, (5, client, 49152));
        assert_eq!(&output[..5], b"query");
    }

    #[test]
    fn flow_index_matches_connected_and_wildcard_sockets_only() {
        let client = Ipv4Address([192, 0, 2, 1]);
        let other_client = Ipv4Address([192, 0, 2, 3]);
        let server = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = UdpSocketTable::new().unwrap();
        let connected = sockets.create().unwrap();
        let wildcard = sockets.create().unwrap();
        let other = sockets.create().unwrap();
        sockets.bind(connected, server, 53).unwrap();
        sockets.connect(connected, client, 49152).unwrap();
        sockets.bind(wildcard, Ipv4Address([0; 4]), 53).unwrap();
        sockets.bind(other, server, 53).unwrap();
        sockets.connect(other, other_client, 49152).unwrap();

        let datagram = UdpDatagram {
            source_port: 49152,
            destination_port: 53,
            payload: b"query",
        };
        assert!(sockets
            .receive_datagram(client, server, &datagram)
            .unwrap());
        assert!(sockets.readable(connected));
        assert!(sockets.readable(wildcard));
        assert!(!sockets.readable(other));
    }

    #[test]
    fn socket_table_grows_beyond_initial_capacity() {
        let mut sockets = UdpSocketTable::new().unwrap();
        for _ in 0..MAX_UDP_SOCKETS {
            sockets.create().unwrap();
        }
        assert_eq!(sockets.slots.len(), MAX_UDP_SOCKETS);
        assert!(matches!(
            sockets.create(),
            Err(crate::kernel::error::Error::NoBufs)
        ));
    }

    #[test]
    fn receive_queue_drops_at_quota_and_recovers_after_dequeue() {
        let client = Ipv4Address([192, 0, 2, 1]);
        let server = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = UdpSocketTable::new().unwrap();
        let socket = sockets.create().unwrap();
        sockets.bind(socket, server, 53).unwrap();
        let mut bytes = [0; 32];
        let length = write_datagram(&mut bytes, client, server, 49152, 53, b"x").unwrap();
        let datagram = UdpDatagram::parse(&bytes[..length], client, server).unwrap();

        for _ in 0..MAX_UDP_RECEIVE_DATAGRAMS_PER_SOCKET {
            assert!(sockets
                .receive_datagram(client, server, &datagram)
                .unwrap());
        }
        assert!(!sockets
            .receive_datagram(client, server, &datagram)
            .unwrap());
        assert_eq!(sockets.queued_datagrams, MAX_UDP_RECEIVE_DATAGRAMS_PER_SOCKET);

        let mut output = [0; 1];
        assert!(sockets.receive(socket, &mut output).unwrap().is_some());
        assert!(sockets
            .receive_datagram(client, server, &datagram)
            .unwrap());
        assert_eq!(sockets.queued_datagrams, MAX_UDP_RECEIVE_DATAGRAMS_PER_SOCKET);
        assert!(sockets.close(socket));
        assert_eq!(sockets.queued_datagrams, 0);
        assert_eq!(sockets.queued_bytes, 0);
    }

    #[test]
    fn receive_queue_drops_at_byte_quota() {
        let client = Ipv4Address([192, 0, 2, 1]);
        let server = Ipv4Address([192, 0, 2, 2]);
        let mut sockets = UdpSocketTable::new().unwrap();
        let socket = sockets.create().unwrap();
        sockets.bind(socket, server, 53).unwrap();
        let payload_length = MAX_UDP_RECEIVE_BYTES_PER_SOCKET / 64 + 1;
        let mut payload = Vec::new();
        payload.resize(payload_length, 0x7a);
        let datagram = UdpDatagram {
            source_port: 49152,
            destination_port: 53,
            payload: &payload,
        };

        let accepted_datagrams = MAX_UDP_RECEIVE_BYTES_PER_SOCKET / payload_length;
        for _ in 0..accepted_datagrams {
            assert!(sockets
                .receive_datagram(client, server, &datagram)
                .unwrap());
        }
        assert!(!sockets
            .receive_datagram(client, server, &datagram)
            .unwrap());
        assert_eq!(
            sockets.queued_bytes,
            accepted_datagrams * payload_length
        );
        assert!(sockets.queued_bytes <= MAX_UDP_RECEIVE_BYTES_PER_SOCKET);
    }
