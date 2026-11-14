/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

/*
 * Augmented red-black interval index; subtree bounds prune point and overlap searches:
 *
 *          [start, end)
 *           /      \
 *       [lower]  [higher]
 */

use core::cmp::Ordering;
use core::ops::Range;

use crate::kernel::lib::btree::{BinaryTree, TreeAugmentation};

#[derive(Clone, Debug, Eq, PartialEq)]
struct IntervalKey(Range<usize>);

impl Ord for IntervalKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .start
            .cmp(&other.0.start)
            .then_with(|| self.0.end.cmp(&other.0.end))
    }
}

impl PartialOrd for IntervalKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy)]
struct IntervalSummary {
    minimum_start: usize,
    maximum_end: usize,
}

struct IntervalAugmentation;

impl<T> TreeAugmentation<IntervalKey, T> for IntervalAugmentation {
    type Summary = IntervalSummary;

    fn summarize(
        key: &IntervalKey,
        _value: &T,
        left: Option<Self::Summary>,
        right: Option<Self::Summary>,
    ) -> Self::Summary {
        let minimum_start = left
            .map(|summary| summary.minimum_start)
            .unwrap_or(key.0.start)
            .min(key.0.start)
            .min(right.map_or(key.0.start, |summary| summary.minimum_start));
        let maximum_end = left
            .map(|summary| summary.maximum_end)
            .unwrap_or(key.0.end)
            .max(key.0.end)
            .max(right.map_or(key.0.end, |summary| summary.maximum_end));
        IntervalSummary {
            minimum_start,
            maximum_end,
        }
    }
}

/// A red-black index of non-empty, half-open intervals keyed by `(start, end)`.
/// Subtrees summarize their minimum start and maximum end to prune point and overlap searches.
/// Intervals may overlap; inserting the same bounds replaces its value. `N` optionally bounds
/// the number of entries and defaults to the largest representable tree length.
pub(crate) struct IntervalTree<T, const N: usize = { usize::MAX }> {
    tree: BinaryTree<IntervalKey, T, IntervalAugmentation>,
}

impl<T, const N: usize> IntervalTree<T, N> {
    pub(crate) const fn new() -> Self {
        Self {
            tree: BinaryTree::new(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.tree.len()
    }

    pub(crate) const fn capacity(&self) -> usize {
        N
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tree.is_empty()
    }

    pub(crate) fn is_full(&self) -> bool {
        self.tree.len() >= N
    }

    /// Inserts an interval, returning its previous value when its bounds existed.
    /// Empty, reversed, or over-capacity inserts return the supplied value.
    pub(crate) fn insert(
        &mut self,
        range: Range<usize>,
        value: T,
    ) -> Result<Option<T>, T> {
        if range.start >= range.end {
            return Err(value);
        }
        let key = IntervalKey(range);
        if self.tree.get(&key).is_none() && self.is_full() {
            return Err(value);
        }
        self.tree
            .try_insert(key, value)
            .map_err(|(_, value)| value)
    }

    pub(crate) fn get(&self, range: &Range<usize>) -> Option<&T> {
        self.tree.get(&IntervalKey(range.clone()))
    }

    pub(crate) fn get_mut(&mut self, range: &Range<usize>) -> Option<&mut T> {
        self.tree.get_mut(&IntervalKey(range.clone()))
    }

    /// Returns the earliest indexed interval containing `point`.
    pub(crate) fn find_point(&self, point: usize) -> Option<(&Range<usize>, &T)> {
        self.tree
            .find_first_by(
                |summary| {
                    summary.minimum_start <= point && summary.maximum_end > point
                },
                |key, _| key.0.start <= point && point < key.0.end,
            )
            .map(|(key, value)| (&key.0, value))
    }

    /// Returns the earliest indexed interval overlapping `query`.
    pub(crate) fn find_overlap(&self, query: Range<usize>) -> Option<(&Range<usize>, &T)> {
        if query.start >= query.end {
            return None;
        }
        self.tree
            .find_first_by(
                |summary| {
                    summary.minimum_start < query.end && summary.maximum_end > query.start
                },
                |key, _| key.0.start < query.end && query.start < key.0.end,
            )
            .map(|(key, value)| (&key.0, value))
    }

    /// Visits every interval in ascending bound order.
    pub(crate) fn visit_all(&self, mut visit: impl FnMut(&Range<usize>, &T)) {
        self.tree
            .for_each(|key, value| visit(&key.0, value));
    }

    /// Finds the first aligned free subrange inside `bounds`.
    /// Indexed intervals are treated as occupied, including overlapping ones.
    pub(crate) fn first_fit(
        &self,
        bounds: Range<usize>,
        length: usize,
        alignment: usize,
    ) -> Option<Range<usize>> {
        if bounds.start >= bounds.end
            || length == 0
            || alignment == 0
            || !alignment.is_power_of_two()
        {
            return None;
        }

        let mut cursor = bounds.start;
        let mut fit = None;
        self.tree.visit_in_order_until(|key, _| {
            let range = &key.0;
            if range.end <= bounds.start {
                return true;
            }
            if range.start >= bounds.end {
                return false;
            }
            let occupied_start = range.start.max(bounds.start);
            let occupied_end = range.end.min(bounds.end);
            if occupied_start > cursor {
                fit = Self::fit_in_gap(cursor, occupied_start, length, alignment);
                if fit.is_some() {
                    return false;
                }
            }
            cursor = cursor.max(occupied_end);
            true
        });
        fit.or_else(|| Self::fit_in_gap(cursor, bounds.end, length, alignment))
    }

    pub(crate) fn remove(&mut self, range: &Range<usize>) -> Option<T> {
        if range.start >= range.end {
            return None;
        }
        self.tree.remove(&IntervalKey(range.clone()))
    }

    pub(crate) fn clear(&mut self) {
        self.tree.clear();
    }

    fn fit_in_gap(
        start: usize,
        end: usize,
        length: usize,
        alignment: usize,
    ) -> Option<Range<usize>> {
        let aligned_start = start.checked_add(alignment - 1)? & !(alignment - 1);
        let allocation_end = aligned_start.checked_add(length)?;
        (allocation_end <= end).then_some(aligned_start..allocation_end)
    }
}

#[cfg(test)]
#[path = "../../tests/kernel/lib/interval.rs"]
mod tests;