/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LookupTable<T, const CAPACITY: usize> {
    entries: [T; CAPACITY],
}

impl<T, const CAPACITY: usize> LookupTable<T, CAPACITY> {
    pub(crate) const fn new(entries: [T; CAPACITY]) -> Self {
        Self { entries }
    }

    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        self.entries.get(index)
    }

    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.entries.get_mut(index)
    }

    pub(crate) fn iter_mut(&mut self) -> core::slice::IterMut<'_, T> {
        self.entries.iter_mut()
    }
}

#[cfg(test)]
#[path = "../../tests/kernel/lib/lookup.rs"]
mod tests;