/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::*;

fn sample_stat_data() -> StatData {
    StatData {
        device: 1,
        inode: 0x1122_3344_5566_7788,
        mode: S_IFREG | 0o640,
        link_count: 2,
        owner: 1000,
        group: 1001,
        size: 1025,
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

#[cfg(target_arch = "x86_64")]
#[test]
fn x86_64_stat_layout_matches_linux_abi() {
    let bytes = encode_stat(sample_stat_data()).unwrap();
    assert_eq!(bytes.len(), 144);
    assert_eq!(read_u64(&bytes, 0), 1);
    assert_eq!(read_u64(&bytes, 8), 0x1122_3344_5566_7788);
    assert_eq!(read_u64(&bytes, 16), 2);
    assert_eq!(read_u32(&bytes, 24), S_IFREG | 0o640);
    assert_eq!(read_u32(&bytes, 28), 1000);
    assert_eq!(read_u32(&bytes, 32), 1001);
    assert_eq!(read_u64(&bytes, 48), 1025);
    assert_eq!(read_u64(&bytes, 56), PREFERRED_BLOCK_SIZE);
    assert_eq!(read_u64(&bytes, 64), 3);
}

#[cfg(target_arch = "aarch64")]
#[test]
fn aarch64_stat_layout_matches_linux_abi() {
    let bytes = encode_stat(sample_stat_data()).unwrap();
    assert_eq!(bytes.len(), 128);
    assert_eq!(read_u64(&bytes, 0), 1);
    assert_eq!(read_u64(&bytes, 8), 0x1122_3344_5566_7788);
    assert_eq!(read_u32(&bytes, 16), S_IFREG | 0o640);
    assert_eq!(read_u32(&bytes, 20), 2);
    assert_eq!(read_u32(&bytes, 24), 1000);
    assert_eq!(read_u32(&bytes, 28), 1001);
    assert_eq!(read_u64(&bytes, 48), 1025);
    assert_eq!(read_u32(&bytes, 56), PREFERRED_BLOCK_SIZE as u32);
    assert_eq!(read_u64(&bytes, 64), 3);
}

#[test]
fn stat_rejects_sizes_outside_signed_linux_file_size_range() {
    let mut data = sample_stat_data();
    data.size = u64::MAX;
    assert_eq!(encode_stat(data), Err(crate::kernel::error::Error::Overflow));
}
