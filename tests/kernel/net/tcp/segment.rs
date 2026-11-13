/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
    use super::{write_segment, TcpSegment, TCP_FLAG_SYN};
    use crate::kernel::net::wire::Ipv4Address;

    #[test]
    fn segment_round_trip_preserves_fields_and_checksum() {
        let source = Ipv4Address([192, 0, 2, 1]);
        let destination = Ipv4Address([192, 0, 2, 2]);
        let mut bytes = [0; 23];
        let length = write_segment(
            &mut bytes,
            source,
            destination,
            49152,
            443,
            7,
            0,
            TCP_FLAG_SYN,
            4096,
            &[1, 2, 3],
        )
        .unwrap();
        let segment = TcpSegment::parse(&bytes[..length], source, destination).unwrap();
        assert_eq!(segment.source_port, 49152);
        assert_eq!(segment.destination_port, 443);
        assert_eq!(segment.sequence_number, 7);
        assert_eq!(segment.flags, TCP_FLAG_SYN);
        assert_eq!(segment.window, 4096);
        assert_eq!(segment.payload, &[1, 2, 3]);
    }
