/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::IntervalTree;

#[test]
fn default_capacity_is_not_application_bounded() {
    let tree = IntervalTree::<u8>::new();

    assert_eq!(tree.capacity(), usize::MAX);
    assert!(!tree.is_full());
}

#[test]
fn point_and_overlap_queries_preserve_half_open_ranges() {
    let mut tree = IntervalTree::<u8, 16>::new();
    assert_eq!(tree.insert(10..40, 1), Ok(None));
    assert_eq!(tree.insert(20..25, 2), Ok(None));
    assert_eq!(tree.insert(50..60, 3), Ok(None));

    let (range, value) = tree.find_point(22).unwrap();
    assert_eq!(range, &(10..40));
    assert_eq!(*value, 1);
    assert!(tree.find_point(40).is_none());

    let (range, value) = tree.find_overlap(24..55).unwrap();
    assert_eq!(range, &(10..40));
    assert_eq!(*value, 1);
    assert!(tree.find_overlap(40..50).is_none());
}

#[test]
fn summaries_stay_valid_after_replacement_and_removal() {
    let mut tree = IntervalTree::<u8, 16>::new();
    for (range, value) in [(0..2, 0), (10..20, 1), (5..100, 2), (30..40, 3)] {
        assert_eq!(tree.insert(range, value), Ok(None));
    }
    assert_eq!(tree.find_point(90).map(|(_, value)| *value), Some(2));

    assert_eq!(tree.remove(&(5..100)), Some(2));
    assert!(tree.find_point(90).is_none());
    assert_eq!(tree.find_point(15).map(|(_, value)| *value), Some(1));
    assert_eq!(tree.insert(10..20, 9), Ok(Some(1)));
    assert_eq!(tree.find_point(15).map(|(_, value)| *value), Some(9));
}

#[test]
fn first_fit_and_capacity_are_preserved() {
    let mut tree = IntervalTree::<(), 3>::new();
    assert_eq!(tree.insert(10..20, ()), Ok(None));
    assert_eq!(tree.insert(15..25, ()), Ok(None));
    assert_eq!(tree.insert(30..35, ()), Ok(None));
    assert!(tree.is_full());
    assert!(tree.insert(40..50, ()).is_err());
    assert_eq!(tree.first_fit(8..50, 8, 8), Some(40..48));
    assert_eq!(tree.remove(&(15..25)), Some(()));
    assert!(!tree.is_full());
}

#[test]
fn empty_and_invalid_ranges_are_rejected() {
    let mut tree = IntervalTree::<u8, 4>::new();

    assert_eq!(tree.insert(5..5, 7), Err(7));
    assert_eq!(tree.insert(6..5, 8), Err(8));
    assert!(tree.get(&(5..5)).is_none());
    assert!(tree.find_overlap(5..5).is_none());
    assert_eq!(tree.len(), 0);
    assert!(tree.is_empty());
}