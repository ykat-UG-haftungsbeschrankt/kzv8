/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::string::String;
use alloc::task::Wake;
use alloc::vec;
use core::task::{Context, Poll, Waker};

use crate::kernel::vfs::{
    DirectoryEntry, FilePlugin, Node, NodeApi as _, VfsFuture, VfsResult,
};

struct EmptyFile;

impl FilePlugin for EmptyFile {
    fn read_at<'a>(
        &'a self,
        _offset: u64,
        _buffer: &'a mut [u8],
    ) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async { Ok(0) })
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_ready<T>(future: &mut VfsFuture<'_, T>) -> T {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("directory future unexpectedly yielded"),
    }
}

#[test]
fn directory_hash_map_supports_lookup_and_listing() {
    let directory = super::directory(vec![DirectoryEntry {
        name: String::from("item"),
        node: Node::file(EmptyFile),
    }])
    .unwrap();
    let Node::Directory { plugin, .. } = directory else {
        panic!("directory builder returned a file node");
    };

    let mut lookup = plugin.lookup("item");
    let found = poll_ready(&mut lookup).unwrap();
    assert!(matches!(found, Node::File { .. }));

    let mut listing = plugin.read_dir();
    let entries = poll_ready(&mut listing).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "item");
}