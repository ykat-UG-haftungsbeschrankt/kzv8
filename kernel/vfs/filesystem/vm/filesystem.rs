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
use crate::kernel::vfs::{
    DirectoryEntry, DirectoryPlugin, FilePlugin, FileSystem, Node, NodeApi as _, NodeMetadata,
    NodeMetadataApi as _, VfsFuture, VfsResult,
};

pub(crate) const HELLO_FILE_NAME: &str = "hello.txt";
pub(crate) const HELLO_FILE_BYTES: &[u8] = b"Hello from the per-VM VirtIO filesystem.\n";

pub(crate) struct VmFileSystem;

impl FileSystem for VmFileSystem {
    fn root(&self) -> VfsResult<Node> {
        let owner = UserId::ROOT;
        let group = GroupId::ROOT;
        let file_metadata = NodeMetadata::with_owner_and_mode(owner, group, 0o444)?;
        let root_metadata = NodeMetadata::with_owner_and_mode(owner, group, 0o555)?;
        let hello_file = Node::file_with_metadata(HelloFile, file_metadata);
        Ok(Node::directory_with_metadata(
            VmRootDirectory { hello_file },
            root_metadata,
        ))
    }
}

struct VmRootDirectory {
    hello_file: Node,
}

impl DirectoryPlugin for VmRootDirectory {
    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, VfsResult<Node>> {
        let result = if name == HELLO_FILE_NAME {
            Ok(self.hello_file.clone())
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
                name: String::from(HELLO_FILE_NAME),
                node: self.hello_file.clone(),
            });
            Ok(entries)
        })();
        Box::pin(async move { result })
    }
}

struct HelloFile;

impl FilePlugin for HelloFile {
    fn read_at<'a>(&'a self, offset: u64, output: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move {
            let Ok(start) = usize::try_from(offset) else {
                return Ok(0);
            };
            let Some(remaining) = HELLO_FILE_BYTES.get(start..) else {
                return Ok(0);
            };
            let length = output.len().min(remaining.len());
            output[..length].copy_from_slice(&remaining[..length]);
            Ok(length)
        })
    }

    fn size(&self) -> VfsFuture<'_, VfsResult<u64>> {
        Box::pin(async { Ok(HELLO_FILE_BYTES.len() as u64) })
    }
}