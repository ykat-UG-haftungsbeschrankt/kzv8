/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::Stack;

#[test]
fn push_and_pop_are_lifo() {
    let mut stack = Stack::new();

    assert!(stack.is_empty());
    assert_eq!(stack.try_push(10), Ok(()));
    assert_eq!(stack.try_push(20), Ok(()));
    assert_eq!(stack.len(), 2);
    assert_eq!(stack.peek(), Some(&20));
    assert_eq!(stack.pop(), Some(20));
    assert_eq!(stack.pop(), Some(10));
    assert_eq!(stack.pop(), None);
    assert!(stack.is_empty());
}

#[test]
fn clear_drops_entries_and_resets_length() {
    let mut stack = Stack::new();
    assert_eq!(stack.try_push(1), Ok(()));
    assert_eq!(stack.try_push(2), Ok(()));

    stack.clear();

    assert!(stack.is_empty());
    assert_eq!(stack.len(), 0);
    assert_eq!(stack.pop(), None);
}