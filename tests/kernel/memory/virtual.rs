/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::VirtualAreaIndex;

#[test]
fn area_index_queries_and_first_fit_follow_sorted_ranges() {
    let mut index = VirtualAreaIndex::new();
    assert_eq!(index.insert(20..28), Ok(None));
    assert_eq!(index.insert(0..10), Ok(None));
    assert_eq!(index.insert(0..10), Ok(Some(())));

    assert!(index.contains_range(&(0..10)));
    assert!(index.find_point(5).is_some());
    assert!(index.find_point(10).is_none());
    assert!(index.find_overlap(9..12).is_some());
    assert!(index.find_overlap(10..20).is_none());
    assert_eq!(index.first_fit(0..40, 4, 4), Some(12..16));

    assert_eq!(index.remove(&(0..10)), Some(()));
    assert!(!index.contains_range(&(0..10)));
    assert_eq!(index.first_fit(0..40, 4, 4), Some(0..4));
}

#[test]
fn area_index_rejects_invalid_ranges_and_alignment() {
    let mut index = VirtualAreaIndex::new();
    assert!(index.insert(10..10).is_err());
    assert_eq!(index.first_fit(0..16, 1, 3), None);
}