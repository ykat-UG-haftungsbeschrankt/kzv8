/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::cmp::Ordering;

pub(crate) struct SortedVector<T> {
    entries: Vec<T>,
}

impl<T> SortedVector<T> {
    pub(crate) fn from_vec_by(
        mut entries: Vec<T>,
        compare: impl FnMut(&T, &T) -> Ordering,
    ) -> Self {
        entries.sort_unstable_by(compare);
        Self { entries }
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        self.entries.get(index)
    }

    pub(crate) fn as_slice(&self) -> &[T] {
        &self.entries
    }

    pub(crate) fn partition_point(&self, predicate: impl FnMut(&T) -> bool) -> usize {
        self.entries.partition_point(predicate)
    }
}

#[cfg(test)]
#[path = "../../tests/kernel/lib/sorted_vector.rs"]
mod tests;