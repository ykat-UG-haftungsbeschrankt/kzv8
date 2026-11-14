/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::kernel::error::Error;
use crate::kernel::vfs::{
    DirectoryEntry, DirectoryPlugin, FilePlugin, FileSystem, Node, NodeApi as _, VfsFuture,
    VfsResult,
};

use super::BlockClientApi;

const SECTOR_SIZE: usize = 512;

pub(crate) struct BlockDeviceFileSystem {
    client: Arc<dyn BlockClientApi>,
}

impl BlockDeviceFileSystem {
    pub(crate) fn new(client: Arc<dyn BlockClientApi>) -> Self {
        Self { client }
    }
}

impl FileSystem for BlockDeviceFileSystem {
    fn root(&self) -> VfsResult<Node> {
        Ok(Node::directory(BlockRootDirectory {
            client: Arc::clone(&self.client),
        }))
    }
}

struct BlockRootDirectory {
    client: Arc<dyn BlockClientApi>,
}

impl DirectoryPlugin for BlockRootDirectory {
    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, VfsResult<Node>> {
        let result = if name == "disk0" {
            Ok(Node::file(BlockDeviceFile {
                client: Arc::clone(&self.client),
            }))
        } else {
            Err(Error::NoEnt)
        };
        Box::pin(async move { result })
    }

    fn read_dir(&self) -> VfsFuture<'_, VfsResult<Vec<DirectoryEntry>>> {
        let result = (|| {
            let mut entries = Vec::new();
            entries.try_reserve_exact(1).map_err(|_| Error::NoMem)?;
            entries.push(DirectoryEntry {
                name: String::from("disk0"),
                node: Node::file(BlockDeviceFile {
                    client: Arc::clone(&self.client),
                }),
            });
            Ok(entries)
        })();
        Box::pin(async move { result })
    }
}

struct BlockDeviceFile {
    client: Arc<dyn BlockClientApi>,
}

impl BlockDeviceFile {
    fn byte_capacity(&self) -> VfsResult<u64> {
        self.client
            .capacity_sectors()
            .checked_mul(SECTOR_SIZE as u64)
            .ok_or(Error::Overflow)
    }

    fn read_at_sync(&self, offset: u64, output: &mut [u8]) -> VfsResult<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let capacity = self.byte_capacity()?;
        if offset >= capacity {
            return Ok(0);
        }
        let requested = u64::try_from(output.len()).map_err(|_| Error::Fbig)?;
        let length = usize::try_from(requested.min(capacity - offset)).map_err(|_| Error::Fbig)?;
        let mut sector_buffer = [0; SECTOR_SIZE];
        let mut completed = 0;
        while completed < length {
            let position = offset
                .checked_add(u64::try_from(completed).map_err(|_| Error::Fbig)?)
                .ok_or(Error::Fbig)?;
            let sector = position / SECTOR_SIZE as u64;
            let sector_offset = (position % SECTOR_SIZE as u64) as usize;
            let amount = (SECTOR_SIZE - sector_offset).min(length - completed);
            let result = self.client.read(sector, &mut sector_buffer);
            if let Err(error) = result {
                return if completed == 0 { Err(error) } else { Ok(completed) };
            }
            output[completed..completed + amount]
                .copy_from_slice(&sector_buffer[sector_offset..sector_offset + amount]);
            completed += amount;
        }
        Ok(completed)
    }

    fn write_at_sync(&self, offset: u64, input: &[u8]) -> VfsResult<usize> {
        if input.is_empty() {
            return Ok(0);
        }
        let capacity = self.byte_capacity()?;
        if offset >= capacity {
            return Ok(0);
        }
        let requested = u64::try_from(input.len()).map_err(|_| Error::Fbig)?;
        let length = usize::try_from(requested.min(capacity - offset)).map_err(|_| Error::Fbig)?;
        let mut sector_buffer = [0; SECTOR_SIZE];
        let mut completed = 0;
        while completed < length {
            let position = offset
                .checked_add(u64::try_from(completed).map_err(|_| Error::Fbig)?)
                .ok_or(Error::Fbig)?;
            let sector = position / SECTOR_SIZE as u64;
            let sector_offset = (position % SECTOR_SIZE as u64) as usize;
            let amount = (SECTOR_SIZE - sector_offset).min(length - completed);
            let result = if sector_offset == 0 && amount == SECTOR_SIZE {
                self.client
                    .write(sector, &input[completed..completed + amount])
            } else {
                self.client.read(sector, &mut sector_buffer).and_then(|()| {
                    sector_buffer[sector_offset..sector_offset + amount]
                        .copy_from_slice(&input[completed..completed + amount]);
                    self.client.write(sector, &sector_buffer)
                })
            };
            if let Err(error) = result {
                return if completed == 0 { Err(error) } else { Ok(completed) };
            }
            completed += amount;
        }
        Ok(completed)
    }
}

impl FilePlugin for BlockDeviceFile {
    fn read_at<'a>(&'a self, offset: u64, output: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move { self.read_at_sync(offset, output) })
    }

    fn write_at<'a>(&'a self, offset: u64, input: &'a [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move { self.write_at_sync(offset, input) })
    }

    fn size(&self) -> VfsFuture<'_, VfsResult<u64>> {
        let size = self.byte_capacity();
        Box::pin(async move { size })
    }
}