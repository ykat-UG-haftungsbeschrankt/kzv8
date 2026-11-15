/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::task::Wake;
use core::task::{Context, Poll, Waker};

use crate::kernel::error::Error;
use crate::kernel::process::Credentials;
use crate::kernel::vfs::{
    AppendResult, DirectoryPlugin, FileHandleApi as _, FileSystem, Node, NodeApi as _, OpenMode,
    SeekFrom, Vfs, VfsApi as _, VfsFuture,
};

use super::RamdiskFileSystem;

struct MountDirectory;

impl DirectoryPlugin for MountDirectory {
    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, Result<Node, Error>> {
        let result = if name == "ramdisk" {
            Ok(Node::directory(EmptyDirectory))
        } else {
            Err(Error::NoEnt)
        };
        Box::pin(async move { result })
    }
}

struct EmptyDirectory;

impl DirectoryPlugin for EmptyDirectory {
    fn lookup<'a>(&'a self, _name: &'a str) -> VfsFuture<'a, Result<Node, Error>> {
        Box::pin(async { Err(Error::NoEnt) })
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
        Poll::Pending => panic!("ramdisk filesystem future unexpectedly yielded"),
    }
}

#[test]
fn ramdisk_filesystem_persists_sparse_writes_and_append() {
    let filesystem: Arc<dyn FileSystem> = Arc::new(RamdiskFileSystem::new());
    let vfs = Vfs::new(Node::directory(MountDirectory));
    let credentials = Credentials::root();
    ready(vfs.mount("/ramdisk", filesystem, &[], &credentials)).unwrap();

    vfs.create_file_blocking("/ramdisk/notes", 0o644, &credentials)
        .unwrap();
    let mut file = vfs
        .open_blocking("/ramdisk/notes", OpenMode::ReadWrite, Credentials::root())
        .unwrap();
    file.seek(SeekFrom::Start(3)).unwrap();
    assert_eq!(ready(file.write(b"ram")).unwrap(), 3);

    let node = ready(vfs.resolve("/ramdisk/notes", &credentials)).unwrap();
    assert_eq!(
        ready(node.append(None, b"-disk")).unwrap(),
        AppendResult {
            end_offset: 11,
            written: 5,
        }
    );

    file.seek(SeekFrom::Start(0)).unwrap();
    let mut contents = [0; 11];
    assert_eq!(ready(file.read(&mut contents)).unwrap(), contents.len());
    assert_eq!(&contents, b"\0\0\0ram-disk");

    let entries = ready(vfs.read_dir("/ramdisk", &credentials)).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "notes");
}