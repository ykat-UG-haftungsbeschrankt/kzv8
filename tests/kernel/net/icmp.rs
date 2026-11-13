/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
    use super::write_fragmentation_needed;
    use crate::kernel::net::wire::internet_checksum;

    #[test]
    fn fragmentation_needed_checksum_covers_next_hop_mtu() {
        let mut original_header = [0; 20];
        original_header[0] = 0x45;
        let mut output = [0; 36];
        let length = write_fragmentation_needed(
            &original_header,
            &[0; 8],
            1400,
            &mut output,
        )
        .unwrap();
        assert_eq!(output[0], 3);
        assert_eq!(output[1], 4);
        assert_eq!(u16::from_be_bytes([output[6], output[7]]), 1400);
        assert_eq!(internet_checksum(&output[..length]), 0);
    }
