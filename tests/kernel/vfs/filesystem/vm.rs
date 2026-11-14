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
    DirectoryPlugin, FileHandleApi as _, FileSystem, Node, NodeApi as _, OpenMode, Vfs,
    VfsApi as _, VfsFuture,
};

use super::VmFileSystem;

struct MountDirectory;

impl DirectoryPlugin for MountDirectory {
    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, Result<Node, Error>> {
        let result = if name == "vm" {
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
        Poll::Pending => panic!("VM filesystem future unexpectedly yielded"),
    }
}

#[test]
fn vm_filesystem_mount_exposes_guest_virtio_fs_contents() {
    let vfs = Vfs::new(Node::directory(MountDirectory));
    let filesystem: Arc<dyn FileSystem> = Arc::new(VmFileSystem);
    let credentials = Credentials::root();
    ready(vfs.mount("/vm", filesystem, &[], &credentials)).unwrap();

    let mut file = vfs
        .open_blocking("/vm/hello.txt", OpenMode::ReadOnly, credentials)
        .unwrap();
    let mut contents = [0; 64];
    let length = ready(file.read(&mut contents)).unwrap();
    assert_eq!(
        &contents[..length],
        b"Hello from the per-VM VirtIO filesystem.\n"
    );
}