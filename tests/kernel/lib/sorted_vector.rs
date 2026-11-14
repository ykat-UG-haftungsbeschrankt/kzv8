/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::SortedVector;

#[derive(Debug, Eq, PartialEq)]
struct Entry {
    key: u8,
    rank: u8,
}

#[test]
fn sorts_entries_and_finds_first_equal_key() {
    let entries = alloc::vec![
        Entry { key: 2, rank: 2 },
        Entry { key: 1, rank: 0 },
        Entry { key: 2, rank: 1 },
    ];
    let sorted = SortedVector::from_vec_by(entries, |left, right| {
        left.key
            .cmp(&right.key)
            .then(left.rank.cmp(&right.rank))
    });

    assert_eq!(
        sorted.as_slice(),
        &[
            Entry { key: 1, rank: 0 },
            Entry { key: 2, rank: 1 },
            Entry { key: 2, rank: 2 },
        ]
    );
    let index = sorted.partition_point(|entry| entry.key < 2);
    assert_eq!(sorted.get(index).map(|entry| entry.rank), Some(1));
    assert_eq!(sorted.get(sorted.len()), None);
    assert!(!sorted.is_empty());
}

#[test]
fn empty_vector_has_zero_lower_bound() {
    let sorted = SortedVector::from_vec_by(alloc::vec::Vec::<u8>::new(), u8::cmp);

    assert!(sorted.is_empty());
    assert_eq!(sorted.len(), 0);
    assert_eq!(sorted.partition_point(|entry| *entry < 1), 0);
    assert_eq!(sorted.get(0), None);
}