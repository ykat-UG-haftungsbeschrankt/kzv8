/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::Queue;
use core::cell::Cell;

#[test]
fn push_and_pop_are_fifo() {
    let mut queue = Queue::new();

    assert!(queue.is_empty());
    assert_eq!(queue.try_push(10), Ok(()));
    assert_eq!(queue.try_push(20), Ok(()));
    assert_eq!(queue.len(), 2);
    assert_eq!(queue.front(), Some(&10));
    assert_eq!(queue.pop(), Some(10));
    assert_eq!(queue.pop(), Some(20));
    assert_eq!(queue.pop(), None);
    assert!(queue.is_empty());
}

#[test]
fn queue_spans_pages_and_can_be_reused() {
    let page_capacity = Queue::<usize>::page_capacity().unwrap();
    let item_count = page_capacity + 2;
    let mut queue = Queue::new();

    for value in 0..item_count {
        assert_eq!(queue.try_push(value), Ok(()));
    }
    for value in 0..item_count {
        assert_eq!(queue.pop(), Some(value));
    }
    assert!(queue.is_empty());

    assert_eq!(queue.try_push(item_count), Ok(()));
    assert_eq!(queue.pop(), Some(item_count));
}

#[test]
fn clear_drops_values_across_pages() {
    struct DropProbe<'a>(&'a Cell<usize>);

    impl Drop for DropProbe<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    let dropped = Cell::new(0);
    let page_capacity = Queue::<DropProbe<'_>>::page_capacity().unwrap();
    let item_count = page_capacity + 1;
    let mut queue = Queue::new();

    for _ in 0..item_count {
        assert!(queue.try_push(DropProbe(&dropped)).is_ok());
    }
    drop(queue.pop());
    assert_eq!(dropped.get(), 1);

    queue.clear();
    assert_eq!(dropped.get(), item_count);
    assert!(queue.is_empty());
}

#[test]
fn zero_sized_values_round_trip() {
    let mut queue = Queue::new();

    assert_eq!(queue.try_push(()), Ok(()));
    assert_eq!(queue.pop(), Some(()));
    assert!(queue.is_empty());
}