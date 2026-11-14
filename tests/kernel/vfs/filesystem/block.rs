/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::task::Wake;
use alloc::vec::Vec;
use core::task::{Context, Poll, Waker};

use crate::kernel::credentials::UserId;
use crate::kernel::error::Error;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::vfs::{
    DirectoryPlugin, FileHandleApi as _, FileSystem as _, Node, NodeApi as _, OpenMode, SeekFrom,
    Vfs, VfsApi as _, VfsFuture,
};

use super::{BlockClientApi, BlockDeviceFileSystem};

const SECTOR_SIZE: usize = 512;

struct MemoryBlockClient {
    bytes: IrqSpinLock<Vec<u8>>,
}

impl BlockClientApi for MemoryBlockClient {
    fn capacity_sectors(&self) -> u64 {
        (self.bytes.lock().len() / SECTOR_SIZE) as u64
    }

    fn smoke(&self) -> Result<(), Error> {
        Ok(())
    }

    fn read(&self, sector: u64, output: &mut [u8]) -> Result<(), Error> {
        if output.is_empty() || output.len() % SECTOR_SIZE != 0 {
            return Err(Error::Inval);
        }
        let start = usize::try_from(sector)
            .ok()
            .and_then(|sector| sector.checked_mul(SECTOR_SIZE))
            .ok_or(Error::Inval)?;
        let end = start.checked_add(output.len()).ok_or(Error::Inval)?;
        let bytes = self.bytes.lock();
        let source = bytes.get(start..end).ok_or(Error::Inval)?;
        output.copy_from_slice(source);
        Ok(())
    }

    fn write(&self, sector: u64, input: &[u8]) -> Result<(), Error> {
        if input.is_empty() || input.len() % SECTOR_SIZE != 0 {
            return Err(Error::Inval);
        }
        let start = usize::try_from(sector)
            .ok()
            .and_then(|sector| sector.checked_mul(SECTOR_SIZE))
            .ok_or(Error::Inval)?;
        let end = start.checked_add(input.len()).ok_or(Error::Inval)?;
        let mut bytes = self.bytes.lock();
        let destination = bytes.get_mut(start..end).ok_or(Error::Inval)?;
        destination.copy_from_slice(input);
        Ok(())
    }
}

struct MountDirectory;

impl DirectoryPlugin for MountDirectory {
    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, Result<Node, Error>> {
        let result = if name == "disk" {
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
        Poll::Pending => panic!("block filesystem future unexpectedly yielded"),
    }
}

#[test]
fn raw_block_filesystem_mounts_and_preserves_partial_sector_writes() {
    let initial: Vec<u8> = (0..SECTOR_SIZE * 2)
        .map(|index| (index % 251) as u8)
        .collect();
    let client: Arc<dyn BlockClientApi> = Arc::new(MemoryBlockClient {
        bytes: IrqSpinLock::new(initial.clone()),
    });
    let filesystem: Arc<dyn crate::kernel::vfs::FileSystem> =
        Arc::new(BlockDeviceFileSystem::new(client));
    let vfs = Vfs::new(Node::directory(MountDirectory));
    let credentials = crate::kernel::process::Credentials::root();
    ready(vfs.mount("/disk", filesystem, &[], &credentials)).unwrap();

    let mut file = vfs
        .open_blocking("/disk/disk0", OpenMode::ReadWrite, credentials)
        .unwrap();
    file.seek(SeekFrom::Start(510)).unwrap();
    let mut across_sector = [0; 4];
    assert_eq!(ready(file.read(&mut across_sector)).unwrap(), across_sector.len());
    assert_eq!(&across_sector, &initial[510..514]);

    file.seek(SeekFrom::Start(509)).unwrap();
    assert_eq!(ready(file.write(b"XY")).unwrap(), 2);
    file.seek(SeekFrom::Start(509)).unwrap();
    let mut after_write = [0; 4];
    assert_eq!(ready(file.read(&mut after_write)).unwrap(), after_write.len());
    assert_eq!(&after_write, &[initial[509], b'X', b'Y', initial[511]]);

    file.seek(SeekFrom::Start((SECTOR_SIZE * 2) as u64)).unwrap();
    let mut end = [0; 1];
    assert_eq!(ready(file.read(&mut end)).unwrap(), 0);
    assert_eq!(file.metadata().owner(), UserId::ROOT);
}