/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::lib::list::LinkedList;

pub(crate) struct Stack<T> {
    entries: LinkedList<T>,
    length: usize,
}

impl<T> Stack<T> {
    pub(crate) const fn new() -> Self {
        Self {
            entries: LinkedList::new(),
            length: 0,
        }
    }

    pub(crate) const fn len(&self) -> usize {
        self.length
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub(crate) fn try_push(&mut self, value: T) -> Result<(), T> {
        let Some(length) = self.length.checked_add(1) else {
            return Err(value);
        };
        self.entries.try_push_front(value)?;
        self.length = length;
        Ok(())
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        let node = self.entries.pop_front()?;
        self.length -= 1;
        Some(node.into_value())
    }

    pub(crate) fn peek(&self) -> Option<&T> {
        self.entries.front()
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.length = 0;
    }
}

#[cfg(test)]
#[path = "../../tests/kernel/lib/stack.rs"]
mod tests;