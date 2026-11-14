/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;
use alloc::task::Wake;
use alloc::vec::Vec;
use core::task::{Context, Poll, Waker};

use crate::kernel::error::Error;
use crate::kernel::process::Credentials;
use crate::kernel::vfs::{
    DirectoryEntry, DirectoryPlugin, FileHandleApi as _, FileSystem as _, Node, NodeApi as _,
    OpenMode, Vfs, VfsApi as _, VfsFuture,
};

struct MountDirectory;

impl DirectoryPlugin for MountDirectory {
    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, Result<Node, Error>> {
        let result = match name {
            "vt1" | "vt2" => Ok(Node::directory(EmptyDirectory)),
            _ => Err(Error::NoEnt),
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

pub(crate) fn reset_for_test() {
    *super::CONSOLE_TTY.lock() = super::ConsoleTtyState::new();
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
        Poll::Pending => panic!("TTY filesystem future unexpectedly yielded"),
    }
}

#[test]
fn filesystem_exposes_console_and_virtual_tty_files() {
    let filesystem = crate::kernel::vfs::filesystem::tty::TtyFileSystem::new_virtual_terminal()
        .unwrap();
    let root = filesystem.root().unwrap();
    let entries: Vec<DirectoryEntry> = ready(root.read_dir()).unwrap();
    assert_eq!(entries.len(), 4);
    assert!(entries.iter().any(|entry| entry.name == "console"));
    assert!(entries.iter().any(|entry| entry.name == "tty"));
    assert!(entries.iter().any(|entry| entry.name == "master"));
    assert!(entries.iter().any(|entry| entry.name == "slave"));
    for entry in entries {
        assert!(matches!(entry.node, Node::File { .. }));
    }
}

#[test]
fn separately_mounted_virtual_terminals_do_not_share_io_state() {
    let credentials = Credentials::root();
    let vfs = Vfs::new(Node::directory(MountDirectory));
    let filesystem_one = super::super::TtyFileSystem::new_virtual_terminal().unwrap();
    let filesystem_two = super::super::TtyFileSystem::new_virtual_terminal().unwrap();
    let filesystem_one: Arc<dyn crate::kernel::vfs::FileSystem> = filesystem_one;
    let filesystem_two: Arc<dyn crate::kernel::vfs::FileSystem> = filesystem_two;

    ready(vfs.mount("/vt1", filesystem_one, &[], &credentials)).unwrap();
    ready(vfs.mount("/vt2", filesystem_two, &[], &credentials)).unwrap();

    let mut master_one = ready(vfs.open("/vt1/master", OpenMode::ReadWrite, Credentials::root()))
        .unwrap();
    let mut slave_one = ready(vfs.open("/vt1/tty", OpenMode::ReadWrite, Credentials::root()))
        .unwrap();
    let mut master_two = ready(vfs.open("/vt2/master", OpenMode::ReadWrite, Credentials::root()))
        .unwrap();
    let mut slave_two = ready(vfs.open("/vt2/tty", OpenMode::ReadWrite, Credentials::root()))
        .unwrap();

    ready(master_one.write(b"first\n")).unwrap();
    ready(master_two.write(b"second\n")).unwrap();
    let mut buffer_one = [0; 16];
    let mut buffer_two = [0; 16];
    let length_one = ready(slave_one.read(&mut buffer_one)).unwrap();
    let length_two = ready(slave_two.read(&mut buffer_two)).unwrap();
    assert_eq!(&buffer_one[..length_one], b"first\n");
    assert_eq!(&buffer_two[..length_two], b"second\n");
    let mut echo = [0; 16];
    let _ = ready(master_one.read(&mut echo)).unwrap();
    let _ = ready(master_two.read(&mut echo)).unwrap();

    ready(slave_one.write(b"reply-one\n")).unwrap();
    let mut output_one = [0; 32];
    let mut output_two = [0; 32];
    let output_length_one = ready(master_one.read(&mut output_one)).unwrap();
    assert_eq!(&output_one[..output_length_one], b"reply-one\r\n");
    assert_eq!(ready(master_two.read(&mut output_two)), Err(Error::Again));

    let mut requested_size = [0; 8];
    requested_size[..2].copy_from_slice(&40u16.to_ne_bytes());
    requested_size[2..4].copy_from_slice(&120u16.to_ne_bytes());
    ready(slave_one.ioctl(0x5414, &mut requested_size, &credentials)).unwrap();
    let mut actual_size_one = [0; 8];
    let mut actual_size_two = [0; 8];
    ready(master_one.ioctl(0x5413, &mut actual_size_one, &credentials)).unwrap();
    ready(master_two.ioctl(0x5413, &mut actual_size_two, &credentials)).unwrap();
    assert_eq!(u16::from_ne_bytes([actual_size_one[0], actual_size_one[1]]), 40);
    assert_eq!(u16::from_ne_bytes([actual_size_one[2], actual_size_one[3]]), 120);
    assert_eq!(u16::from_ne_bytes([actual_size_two[0], actual_size_two[1]]), 25);
    assert_eq!(u16::from_ne_bytes([actual_size_two[2], actual_size_two[3]]), 80);

    let mut attributes_one = [0; 56];
    let mut attributes_two = [0; 56];
    ready(slave_one.ioctl(
        crate::kernel::vfs::filesystem::tty::TCGETS,
        &mut attributes_one,
        &credentials,
    ))
    .unwrap();
    attributes_one[12..16].copy_from_slice(&0u32.to_ne_bytes());
    ready(slave_one.ioctl(
        crate::kernel::vfs::filesystem::tty::TCSETS,
        &mut attributes_one,
        &credentials,
    ))
    .unwrap();
    ready(slave_one.ioctl(
        crate::kernel::vfs::filesystem::tty::TCGETS,
        &mut attributes_one,
        &credentials,
    ))
    .unwrap();
    ready(slave_two.ioctl(
        crate::kernel::vfs::filesystem::tty::TCGETS,
        &mut attributes_two,
        &credentials,
    ))
    .unwrap();
    assert_eq!(u32::from_ne_bytes(attributes_one[12..16].try_into().unwrap()), 0);
    assert_ne!(u32::from_ne_bytes(attributes_two[12..16].try_into().unwrap()), 0);
}