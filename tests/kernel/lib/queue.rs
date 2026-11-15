/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{Queue, QueueTyped};
use alloc::vec;
use alloc::vec::Vec;

#[test]
fn byte_writes_can_be_peeked_and_read() {
    let mut queue = Queue::new();

    assert!(queue.is_empty());
    assert_eq!(queue.write(b"hello"), 5);
    assert_eq!(queue.len(), 5);

    let mut peeked = [0; 3];
    assert_eq!(queue.peek(&mut peeked), 3);
    assert_eq!(&peeked, b"hel");
    assert_eq!(queue.len(), 5);

    let mut first = [0; 2];
    assert_eq!(queue.read(&mut first), 2);
    assert_eq!(&first, b"he");
    let mut rest = [0; 8];
    assert_eq!(queue.read(&mut rest), 3);
    assert_eq!(&rest[..3], b"llo");
    assert_eq!(queue.read(&mut rest), 0);
    assert!(queue.is_empty());
}

#[test]
fn byte_queue_spans_pages_and_can_be_reused() {
    let page_capacity = Queue::page_capacity().unwrap();
    let bytes = (0..page_capacity + 3)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let mut queue = Queue::new();

    assert_eq!(queue.write(&bytes), bytes.len());
    let mut peeked = vec![0; bytes.len()];
    assert_eq!(queue.peek(&mut peeked), bytes.len());
    assert_eq!(peeked, bytes);
    let mut read = vec![0; bytes.len()];
    assert_eq!(queue.read(&mut read), bytes.len());
    assert_eq!(read, bytes);
    assert!(queue.is_empty());

    assert_eq!(queue.write(b"reuse"), 5);
    let mut reused = [0; 5];
    assert_eq!(queue.read(&mut reused), 5);
    assert_eq!(&reused, b"reuse");
}

#[test]

fn typed_queue_reads_fixed_size_values() {
    let item_count = Queue::page_capacity().unwrap() / 3 + 2;
    let mut queue = QueueTyped::<[u8; 3]>::new();

    for index in 0..item_count {
        let value = [(index % 251) as u8; 3];
        assert_eq!(queue.write(value), Ok(()));
    }
    assert_eq!(queue.len(), item_count);

    for index in 0..item_count {
        let value = [(index % 251) as u8; 3];
        assert_eq!(queue.peek(), Some(value));
        assert_eq!(queue.read(), Some(value));
    }
    assert_eq!(queue.read(), None);
    assert!(queue.is_empty());
}

#[test]
fn typed_queue_rejects_zero_sized_values() {
    let mut queue = QueueTyped::<()>::new();
    assert_eq!(queue.write(()), Err(()));
    assert!(queue.is_empty());
}