/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
    use super::{
        fragment_ipv4_packet, rewrite_forwarded_ipv4_header, write_ipv4_header, Ipv4Address,
        Ipv4Packet, IPV4_PROTOCOL_UDP,
    };
    use crate::kernel::error::Error;
    use crate::kernel::net::ipv4_fragments::Ipv4FragmentReassembler;

    #[test]
    fn forwarding_decrements_ttl_and_repairs_checksum() {
        let source = Ipv4Address([192, 0, 2, 1]);
        let destination = Ipv4Address([198, 51, 100, 8]);
        let mut original = [0; 20];
        write_ipv4_header(
            &mut original,
            source,
            destination,
            IPV4_PROTOCOL_UDP,
            64,
            17,
            0,
            0,
        )
        .unwrap();
        let mut forwarded = [0; 20];
        rewrite_forwarded_ipv4_header(&mut forwarded, &original).unwrap();
        let packet = Ipv4Packet::parse(&forwarded).unwrap();
        assert_eq!(packet.ttl, 63);
        assert_eq!(packet.source, source);
        assert_eq!(packet.destination, destination);
    }

    #[test]
    fn ipv4_fragments_reassemble_out_of_order() {
        let source = Ipv4Address([192, 0, 2, 1]);
        let destination = Ipv4Address([198, 51, 100, 8]);
        let mut original = [0; 60];
        write_ipv4_header(
            &mut original,
            source,
            destination,
            IPV4_PROTOCOL_UDP,
            64,
            41,
            0,
            40,
        )
        .unwrap();
        for (index, byte) in original[20..].iter_mut().enumerate() {
            *byte = index as u8;
        }
        let packet = Ipv4Packet::parse(&original).unwrap();
        let fragments = fragment_ipv4_packet(&packet, 36).unwrap();
        assert_eq!(fragments.len(), 3);

        let mut reassembler = Ipv4FragmentReassembler::new();
        let mut complete = None;
        for (index, bytes) in fragments.iter().rev().enumerate() {
            let fragment = Ipv4Packet::parse(bytes).unwrap();
            complete = reassembler
                .insert(&fragment, index as u64)
                .unwrap()
                .or(complete);
        }
        let reassembled = complete.unwrap();
        let parsed = Ipv4Packet::parse(&reassembled).unwrap();
        assert_eq!(parsed.source, source);
        assert_eq!(parsed.destination, destination);
        assert_eq!(parsed.identification, 41);
        assert_eq!(parsed.fragment_offset, 0);
        assert!(!parsed.more_fragments);
        assert_eq!(parsed.payload, &original[20..]);
    }

    #[test]
    fn ipv4_fragmenter_honors_dont_fragment() {
        let source = Ipv4Address([192, 0, 2, 1]);
        let destination = Ipv4Address([198, 51, 100, 8]);
        let mut original = [0; 60];
        write_ipv4_header(
            &mut original,
            source,
            destination,
            IPV4_PROTOCOL_UDP,
            64,
            42,
            0x4000,
            40,
        )
        .unwrap();
        let packet = Ipv4Packet::parse(&original).unwrap();
        assert_eq!(
            fragment_ipv4_packet(&packet, 36),
            Err(Error::MsgSize)
        );
    }

    #[test]
    fn fragment_reassembly_expires_from_first_arrival() {
        let source = Ipv4Address([192, 0, 2, 1]);
        let destination = Ipv4Address([198, 51, 100, 8]);
        let mut original = [0; 60];
        write_ipv4_header(
            &mut original,
            source,
            destination,
            IPV4_PROTOCOL_UDP,
            64,
            43,
            0,
            40,
        )
        .unwrap();
        let packet = Ipv4Packet::parse(&original).unwrap();
        let fragments = fragment_ipv4_packet(&packet, 36).unwrap();
        let first = Ipv4Packet::parse(&fragments[0]).unwrap();
        let middle = Ipv4Packet::parse(&fragments[1]).unwrap();
        let last = Ipv4Packet::parse(fragments.last().unwrap()).unwrap();
        let mut reassembler = Ipv4FragmentReassembler::new();

        assert!(reassembler.insert(&first, 0).unwrap().is_none());
        assert!(reassembler
            .insert(&middle, 20_000_000_000)
            .unwrap()
            .is_none());
        assert!(reassembler
            .insert(&last, 31_000_000_000)
            .unwrap()
            .is_none());
    }
