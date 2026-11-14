/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::LookupTable;

#[test]
fn lookup_table_reads_by_index_and_returns_none_outside_capacity() {
    let table = LookupTable::new([10, 20, 30]);

    assert_eq!(table.get(0).copied(), Some(10));
    assert_eq!(table.get(2).copied(), Some(30));
    assert_eq!(table.get(3), None);
}

#[test]
fn lookup_table_mutates_entries_through_checked_access_and_iteration() {
    let mut table = LookupTable::new([1, 2, 3]);

    *table.get_mut(1).unwrap() = 20;
    for value in table.iter_mut() {
        *value += 1;
    }

    assert_eq!(table.get(0).copied(), Some(2));
    assert_eq!(table.get(1).copied(), Some(21));
    assert_eq!(table.get(2).copied(), Some(4));
    assert!(table.get_mut(3).is_none());
}