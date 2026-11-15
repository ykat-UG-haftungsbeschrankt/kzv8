/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::kernel::credentials::{GroupId, UserId};
use crate::kernel::error::Error;
use crate::kernel::lib::hashmap::HashMap;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::vfs::{
    AppendResult, DirectoryEntry, DirectoryPlugin, FilePlugin, FileSystem, Node, NodeApi as _,
    NodeMetadata, NodeMetadataApi as _, VfsFuture, VfsResult,
};

pub(crate) struct RamdiskFileSystem {
    root: Node,
}

impl RamdiskFileSystem {
    pub(crate) fn new() -> Self {
        Self {
            root: Node::directory(RamdiskDirectory {
                entries: IrqSpinLock::new(HashMap::new()),
            }),
        }
    }
}

impl FileSystem for RamdiskFileSystem {
    fn root(&self) -> VfsResult<Node> {
        Ok(self.root.clone())
    }
}

struct RamdiskDirectory {
    entries: IrqSpinLock<HashMap<String, Node, 8>>,
}

impl DirectoryPlugin for RamdiskDirectory {
    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, VfsResult<Node>> {
        let result = self.entries.lock().get(name).cloned().ok_or(Error::NoEnt);
        Box::pin(async move { result })
    }

    fn create_file<'a>(&'a self, name: &'a str, mode: u16) -> VfsFuture<'a, VfsResult<()>> {
        let result = (|| {
            validate_entry_name(name)?;
            let metadata = NodeMetadata::with_owner_and_mode(
                UserId::ROOT,
                GroupId::ROOT,
                mode & 0o777,
            )?;
            self.insert_entry(
                name,
                Node::file_with_metadata(
                    RamdiskFile {
                        contents: IrqSpinLock::new(Vec::new()),
                    },
                    metadata,
                ),
            )
        })();
        Box::pin(async move { result })
    }

    fn insert_entry(&self, name: &str, node: Node) -> VfsResult<()> {
        validate_entry_name(name)?;
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

    fn remove_file_entry(&self, name: &str) -> VfsResult<Node> {
        let mut entries = self.entries.lock();
        let node = entries.get(name).ok_or(Error::NoEnt)?;
        if matches!(node, Node::Directory { .. }) {
            return Err(Error::IsDir);
        }
        entries.remove(name).ok_or(Error::NoEnt)
    }

    fn read_dir(&self) -> VfsFuture<'_, VfsResult<Vec<DirectoryEntry>>> {
        let result = (|| {
            let mut entries = Vec::new();
            let directory = self.entries.lock();
            entries
                .try_reserve_exact(directory.len())
                .map_err(|_| Error::NoMem)?;
            directory.try_for_each(|name, node| {
                let mut name_copy = String::new();
                name_copy
                    .try_reserve_exact(name.len())
                    .map_err(|_| Error::NoMem)?;
                name_copy.push_str(name);
                entries.push(DirectoryEntry {
                    name: name_copy,
                    node: node.clone(),
                });
                Ok::<(), Error>(())
            })?;
            Ok(entries)
        })();
        Box::pin(async move { result })
    }
}

fn validate_entry_name(name: &str) -> VfsResult<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        Err(Error::Inval)
    } else {
        Ok(())
    }
}

struct RamdiskFile {
    contents: IrqSpinLock<Vec<u8>>,
}

impl RamdiskFile {
    fn read_at_sync(&self, offset: u64, output: &mut [u8]) -> VfsResult<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let Ok(start) = usize::try_from(offset) else {
            return Ok(0);
        };
        let contents = self.contents.lock();
        let Some(remaining) = contents.get(start..) else {
            return Ok(0);
        };
        let length = output.len().min(remaining.len());
        output[..length].copy_from_slice(&remaining[..length]);
        Ok(length)
    }

    fn write_at_sync(&self, offset: u64, input: &[u8]) -> VfsResult<usize> {
        if input.is_empty() {
            return Ok(0);
        }
        let start = usize::try_from(offset).map_err(|_| Error::Fbig)?;
        let end = start.checked_add(input.len()).ok_or(Error::Fbig)?;
        let mut contents = self.contents.lock();
        let additional = end.saturating_sub(contents.len());
        if additional != 0 {
            contents.try_reserve(additional).map_err(|_| Error::NoMem)?;
            contents.resize(end, 0);
        }
        contents[start..end].copy_from_slice(input);
        Ok(input.len())
    }

    fn append_sync(&self, input: &[u8]) -> VfsResult<AppendResult> {
        let mut contents = self.contents.lock();
        let end = contents
            .len()
            .checked_add(input.len())
            .ok_or(Error::Fbig)?;
        let end_offset = u64::try_from(end).map_err(|_| Error::Fbig)?;
        if !input.is_empty() {
            contents
                .try_reserve(input.len())
                .map_err(|_| Error::NoMem)?;
            contents.extend_from_slice(input);
        }
        Ok(AppendResult {
            end_offset,
            written: input.len(),
        })
    }
}

impl FilePlugin for RamdiskFile {
    fn read_at<'a>(&'a self, offset: u64, output: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move { self.read_at_sync(offset, output) })
    }

    fn size(&self) -> VfsFuture<'_, VfsResult<u64>> {
        let size = u64::try_from(self.contents.lock().len()).map_err(|_| Error::Fbig);
        Box::pin(async move { size })
    }

    fn write_at<'a>(&'a self, offset: u64, input: &'a [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move { self.write_at_sync(offset, input) })
    }

    fn append<'a>(&'a self, input: &'a [u8]) -> VfsFuture<'a, VfsResult<AppendResult>> {
        Box::pin(async move { self.append_sync(input) })
    }
}

#[cfg(test)]
#[path = "../../../tests/kernel/vfs/filesystem/ramdisk.rs"]
mod tests;