/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::*;

#[test]
fn directory_record_uses_linux_dirent64_layout() {
    let mut output = Vec::new();
    assert!(append_dirent64(&mut output, 24, "abc", 0x1234, 7, DT_REG).unwrap());
    assert_eq!(output.len(), 24);
    assert_eq!(u64::from_le_bytes(output[0..8].try_into().unwrap()), 0x1234);
    assert_eq!(i64::from_le_bytes(output[8..16].try_into().unwrap()), 7);
    assert_eq!(u16::from_le_bytes(output[16..18].try_into().unwrap()), 24);
    assert_eq!(output[18], DT_REG);
    assert_eq!(&output[19..23], b"abc\0");
    assert_eq!(output[23], 0);
}

#[test]
fn directory_record_rejects_too_small_buffer_without_partial_entry() {
    let mut output = Vec::new();
    assert_eq!(
        append_dirent64(&mut output, 23, "x", 1, 1, DT_DIR),
        Err(crate::kernel::error::Error::Inval)
    );
    assert!(output.is_empty());
}

#[test]
fn directory_records_stop_at_buffer_boundary() {
    let mut output = Vec::new();
    assert!(append_dirent64(&mut output, 24, "x", 1, 1, DT_DIR).unwrap());
    assert!(!append_dirent64(&mut output, 24, "y", 2, 2, DT_LNK).unwrap());
    assert_eq!(output.len(), 24);
}