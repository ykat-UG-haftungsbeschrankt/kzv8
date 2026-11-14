/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::error::Error;
use crate::kernel::process::Credentials;
use crate::kernel::lib::hashmap::HashMap;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::vfs::{
    DirectoryPlugin, FilePlugin, Node, NodeApi as _, NodeMetadata, NodeMetadataApi as _, Vfs,
    VfsApi as _, VfsFuture,
};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::task::Wake;
use alloc::vec;
use alloc::vec::Vec;
use core::task::{Context, Poll, Waker};

struct TestDirectory {
    entries: IrqSpinLock<HashMap<String, Node>>,
}

impl TestDirectory {
    fn new() -> Self {
        Self {
            entries: IrqSpinLock::new(HashMap::new()),
        }
    }
}

impl DirectoryPlugin for TestDirectory {
    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, Result<Node, Error>> {
        let result = self.entries.lock().get(name).cloned().ok_or(Error::NoEnt);
        Box::pin(async move { result })
    }

    fn insert_entry(&self, name: &str, node: Node) -> Result<(), Error> {
        let mut owned_name = String::new();
        owned_name
            .try_reserve_exact(name.len())
            .map_err(|_| Error::NoMem)?;
        owned_name.push_str(name);
        let mut entries = self.entries.lock();
        if entries.contains_key(name) {
            return Err(Error::Exist);
        }
        entries
            .insert(owned_name, node)
            .map_err(|(_name, _node)| Error::NoMem)?;
        Ok(())
    }

    fn remove_file_entry(&self, name: &str) -> Result<Node, Error> {
        let mut entries = self.entries.lock();
        if matches!(entries.get(name), Some(Node::Directory { .. })) {
            return Err(Error::IsDir);
        }
        entries.remove(name).ok_or(Error::NoEnt)
    }
}

struct EmptyFile;

impl FilePlugin for EmptyFile {
    fn read_at<'a>(
        &'a self,
        _offset: u64,
        _buffer: &'a mut [u8],
    ) -> VfsFuture<'a, Result<usize, Error>> {
        Box::pin(async { Ok(0) })
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn ready<T>(mut future: VfsFuture<'_, T>) -> T {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("VFS test future unexpectedly yielded"),
    }
}

fn directory(entries: Vec<(&str, Node)>) -> Node {
    let directory = Node::directory(TestDirectory::new());
    let Node::Directory { plugin, .. } = &directory else {
        panic!("directory constructor returned a non-directory");
    };
    for (name, node) in entries {
        plugin.insert_entry(name, node).unwrap();
    }
    directory
}

#[test]
fn xattr_hash_map_preserves_create_replace_get_list_remove() {
    let metadata = NodeMetadata::new();
    let credentials = Credentials::root();
    let name = b"user.label";

    metadata.set_xattr(&credentials, name, b"first", 0).unwrap();
    let first_value = metadata.get_xattr(name).unwrap();
    assert_eq!(first_value.as_bytes(), b"first");
    let same_value = metadata.get_xattr(name).unwrap();
    assert!(Arc::ptr_eq(&first_value, &same_value));
    let listed = metadata.list_xattrs().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].as_bytes(), name);
    let listed_again = metadata.list_xattrs().unwrap();
    assert!(Arc::ptr_eq(&listed[0], &listed_again[0]));

    assert_eq!(
        metadata.set_xattr(&credentials, name, b"duplicate", super::XATTR_CREATE),
        Err(Error::Exist)
    );
    metadata
        .set_xattr(&credentials, name, b"replacement", super::XATTR_REPLACE)
        .unwrap();
    let replacement = metadata.get_xattr(name).unwrap();
    assert_eq!(replacement.as_bytes(), b"replacement");
    assert!(!Arc::ptr_eq(&first_value, &replacement));
    assert_eq!(
        metadata.set_xattr(&credentials, b"user.missing", b"value", super::XATTR_REPLACE),
        Err(Error::NoData)
    );

    metadata.remove_xattr(&credentials, name).unwrap();
    assert_eq!(metadata.get_xattr(name).map(|_| ()), Err(Error::NoData));
    assert!(metadata.list_xattrs().unwrap().is_empty());
}

#[test]
fn hard_links_share_file_identity_and_unlink_updates_link_count() {
    let credentials = Credentials::root();
    let original = Node::file(EmptyFile);
    let vfs = Vfs::new(directory(vec![("original", original)]));

    ready(vfs.hard_link("/original", "/alias", &credentials)).unwrap();
    let original = ready(vfs.resolve("/original", &credentials)).unwrap();
    let alias = ready(vfs.resolve("/alias", &credentials)).unwrap();
    let (Node::File {
        plugin: original_plugin,
        metadata: original_metadata,
    }, Node::File {
        plugin: alias_plugin,
        metadata: alias_metadata,
    }) = (&original, &alias)
    else {
        panic!("hard link did not resolve to files");
    };
    assert!(Arc::ptr_eq(original_plugin, alias_plugin));
    assert!(Arc::ptr_eq(&original_metadata.state, &alias_metadata.state));
    assert_eq!(original_metadata.link_count(), 2);
    assert_eq!(
        ready(vfs.hard_link("/original", "/alias", &credentials)),
        Err(Error::Exist)
    );

    ready(vfs.unlink("/alias", &credentials)).unwrap();
    assert_eq!(original_metadata.link_count(), 1);
    assert_eq!(ready(vfs.resolve("/alias", &credentials)).map(|_| ()), Err(Error::NoEnt));
    assert!(ready(vfs.resolve("/original", &credentials)).is_ok());
}

#[test]
fn symbolic_links_follow_targets_and_detect_cycles() {
    let credentials = Credentials::root();
    let file = Node::file(EmptyFile);
    let vfs = Vfs::new(directory(vec![
        ("file", file),
        ("dir", directory(Vec::new())),
    ]));

    ready(vfs.symlink("file", "/relative", &credentials)).unwrap();
    ready(vfs.symlink("/file", "/absolute", &credentials)).unwrap();
    ready(vfs.symlink("../file", "/dir/up", &credentials)).unwrap();
    assert_eq!(ready(vfs.read_link("/relative", &credentials)).unwrap(), "file");
    assert_eq!(ready(vfs.read_link("/absolute", &credentials)).unwrap(), "/file");

    let target = ready(vfs.resolve("/file", &credentials)).unwrap();
    for path in ["/relative", "/absolute", "/dir/up"] {
        let linked = ready(vfs.resolve(path, &credentials)).unwrap();
        let (Node::File { plugin: target_plugin, .. }, Node::File { plugin: linked_plugin, .. }) =
            (&target, &linked)
        else {
            panic!("symbolic link did not resolve to a file");
        };
        assert!(Arc::ptr_eq(target_plugin, linked_plugin));
    }

    ready(vfs.symlink("loop-b", "/loop-a", &credentials)).unwrap();
    ready(vfs.symlink("loop-a", "/loop-b", &credentials)).unwrap();
    assert_eq!(
        ready(vfs.resolve("/loop-a", &credentials)).map(|_| ()),
        Err(Error::Loop)
    );
    assert_eq!(ready(vfs.read_link("/file", &credentials)).map(|_| ()), Err(Error::Inval));
    ready(vfs.unlink("/relative", &credentials)).unwrap();
    assert_eq!(ready(vfs.read_link("/relative", &credentials)), Err(Error::NoEnt));
}

#[test]
fn mounts_are_shared_and_nested_mounts_must_be_removed_first() {
    let credentials = Credentials::root();
    let root = directory(vec![(
        "mnt",
        directory(vec![("covered", Node::file(EmptyFile))]),
    )]);
    let mounted_root = directory(vec![
        ("inside", Node::file(EmptyFile)),
        ("nested", directory(Vec::new())),
    ]);
    let vfs = Vfs::new(root);
    let clone = vfs.clone();

    ready(vfs.mount("/mnt", mounted_root, &credentials)).unwrap();
    assert!(ready(clone.resolve("/mnt/inside", &credentials)).is_ok());
    assert_eq!(
        ready(clone.resolve("/mnt/covered", &credentials)).map(|_| ()),
        Err(Error::NoEnt)
    );
    assert_eq!(
        ready(vfs.mount("/mnt", directory(Vec::new()), &credentials)),
        Err(Error::Busy)
    );
    assert_eq!(
        ready(vfs.hard_link("/mnt/inside", "/outside-link", &credentials)),
        Err(Error::Xdev)
    );

    ready(vfs.mount("/mnt/nested", directory(Vec::new()), &credentials)).unwrap();
    assert_eq!(ready(vfs.unmount("/mnt", &credentials)), Err(Error::Busy));
    ready(vfs.umount("/mnt/nested", &credentials)).unwrap();
    ready(vfs.unmount("/mnt", &credentials)).unwrap();
    assert!(ready(vfs.resolve("/mnt/covered", &credentials)).is_ok());
    assert_eq!(
        ready(vfs.resolve("/mnt/inside", &credentials)).map(|_| ()),
        Err(Error::NoEnt)
    );
}