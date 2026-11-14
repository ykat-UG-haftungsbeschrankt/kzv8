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
use crate::kernel::lib::immutable_byte_buffer::ImmutableByteBuffer;
use core::future::Future;
use core::pin::Pin;

use crate::kernel::error::Error;
use crate::kernel::process::{Credentials, GroupId, UserId};
use crate::kernel::scheduler::sync::IrqSpinLock;

pub(crate) mod plugin;
mod vfs;

pub(crate) use vfs::mounted;
#[allow(unused_imports)]
pub(crate) use vfs::mount_virtio_devices;

pub(crate) type VfsResult<T> = Result<T, Error>;
pub(crate) type VfsFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Result of an atomic append, including the resulting file offset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AppendResult {
    pub(crate) end_offset: u64,
    pub(crate) written: usize,
}

pub(crate) const INODE_FLAG_IMMUTABLE: u32 = 0x0000_0010;
pub(crate) const INODE_FLAG_APPEND: u32 = 0x0000_0020;
pub(crate) const INODE_FLAG_NODUMP: u32 = 0x0000_0040;
pub(crate) const INODE_FLAG_NOATIME: u32 = 0x0000_0080;
pub(crate) const INODE_FLAG_DIRSYNC: u32 = 0x0001_0000;
pub(crate) const INODE_FLAG_TOPDIR: u32 = 0x0002_0000;

#[derive(Clone)]
pub(crate) struct NodeMetadata {
    state: Arc<IrqSpinLock<vfs::NodeMetadataState>>,
}

pub(crate) trait NodeMetadataApi: Sized {
    fn new() -> Self;
    fn with_owner_and_mode(owner: UserId, group: GroupId, mode: u16) -> VfsResult<Self>;
    fn owner(&self) -> UserId;
    fn group(&self) -> GroupId;
    fn mode(&self) -> u16;
    fn check_access(
        &self,
        credentials: &Credentials,
        requested: u16,
        is_directory: bool,
    ) -> VfsResult<()>;
    fn set_mode(&self, credentials: &Credentials, mode: u16) -> VfsResult<()>;
    fn set_owner_and_group(
        &self,
        credentials: &Credentials,
        owner: UserId,
        group: GroupId,
    ) -> VfsResult<()>;
    fn update_from_filesystem(&self, owner: UserId, group: GroupId, mode: u16) -> VfsResult<()>;
    fn flags(&self) -> u32;
    fn set_flags(&self, credentials: &Credentials, flags: u32) -> VfsResult<()>;
    fn get_xattr(&self, name: &[u8]) -> VfsResult<Arc<ImmutableByteBuffer>>;
    fn list_xattrs(&self) -> VfsResult<Vec<Arc<ImmutableByteBuffer>>>;
    fn set_xattr(
        &self,
        credentials: &Credentials,
        name: &[u8],
        value: &[u8],
        flags: i32,
    ) -> VfsResult<()>;
    fn remove_xattr(&self, credentials: &Credentials, name: &[u8]) -> VfsResult<()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OpenMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SeekFrom {
    Start(u64),
    Current(i64),
}

pub(crate) trait DirectoryPlugin: Send + Sync {
    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, VfsResult<Node>>;

    fn read_dir(&self) -> VfsFuture<'_, VfsResult<Vec<DirectoryEntry>>> {
        Box::pin(async { Err(Error::NoSys) })
    }
}

pub(crate) trait FilePlugin: Send + Sync {
    fn open(&self, _mode: OpenMode) -> VfsFuture<'_, VfsResult<Option<Arc<dyn FilePlugin>>>> {
        Box::pin(async { Ok(None) })
    }

    fn read_at<'a>(&'a self, offset: u64, buffer: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>>;

    fn size(&self) -> VfsFuture<'_, VfsResult<u64>> {
        Box::pin(async { Err(Error::NoSys) })
    }

    fn write_at<'a>(&'a self, _offset: u64, _buffer: &'a [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async { Err(Error::Rofs) })
    }

    /// Appends at EOF atomically with other writes to this file.
    fn append<'a>(&'a self, _buffer: &'a [u8]) -> VfsFuture<'a, VfsResult<AppendResult>> {
        Box::pin(async { Err(Error::NoSys) })
    }
}

#[derive(Clone)]
pub(crate) enum Node {
    Directory {
        plugin: Arc<dyn DirectoryPlugin>,
        metadata: NodeMetadata,
    },
    File {
        plugin: Arc<dyn FilePlugin>,
        metadata: NodeMetadata,
    },
}

pub(crate) trait DirectoryHandleReference: Send + Sync {
    fn path(&self) -> VfsResult<Vec<u8>>;
}

#[derive(Clone)]
pub(crate) struct DirectoryHandle {
    reference: Arc<dyn DirectoryHandleReference>,
}

struct RootDirectoryHandleReference;

impl DirectoryHandleReference for RootDirectoryHandleReference {
    fn path(&self) -> VfsResult<Vec<u8>> {
        let mut path = Vec::new();
        path.try_reserve_exact(1).map_err(|_| Error::NoMem)?;
        path.push(b'/');
        Ok(path)
    }
}

impl DirectoryHandle {
    pub(crate) fn root() -> Self {
        Self {
            reference: Arc::new(RootDirectoryHandleReference),
        }
    }

    pub(crate) fn new(reference: impl DirectoryHandleReference + 'static) -> Self {
        Self {
            reference: Arc::new(reference),
        }
    }

    pub(crate) fn path(&self) -> VfsResult<Vec<u8>> {
        self.reference.path()
    }
}

pub(crate) trait NodeApi: Sized {
    fn directory<P: DirectoryPlugin + 'static>(plugin: P) -> Self;
    fn file<P: FilePlugin + 'static>(plugin: P) -> Self;
    fn directory_with_metadata<P: DirectoryPlugin + 'static>(
        plugin: P,
        metadata: NodeMetadata,
    ) -> Self;
    fn file_with_metadata<P: FilePlugin + 'static>(plugin: P, metadata: NodeMetadata) -> Self;
    fn metadata(&self) -> NodeMetadata;
    fn flags(&self) -> u32;
    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, VfsResult<Node>>;
    fn read_dir(&self) -> VfsFuture<'_, VfsResult<Vec<DirectoryEntry>>>;
    fn read_at<'a>(
        &'a self,
        offset: u64,
        buffer: &'a mut [u8],
    ) -> VfsFuture<'a, VfsResult<usize>>;
    fn size(&self) -> VfsFuture<'_, VfsResult<u64>>;
    fn write_at<'a>(
        &'a self,
        offset: u64,
        buffer: &'a [u8],
    ) -> VfsFuture<'a, VfsResult<usize>>;
    fn append<'a>(
        &'a self,
        opened_plugin: Option<&'a Arc<dyn FilePlugin>>,
        buffer: &'a [u8],
    ) -> VfsFuture<'a, VfsResult<AppendResult>>;
}

pub(crate) struct FileHandle {
    node: Node,
    opened_plugin: Option<Arc<dyn FilePlugin>>,
    mode: OpenMode,
    offset: u64,
}

pub(crate) trait FileHandleApi: Sized {
    fn read_blocking(&mut self, buffer: &mut [u8]) -> VfsResult<usize>;
    fn read<'a>(&'a mut self, buffer: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>>;
    fn write_blocking(&mut self, buffer: &[u8]) -> VfsResult<usize>;
    fn write<'a>(&'a mut self, buffer: &'a [u8]) -> VfsFuture<'a, VfsResult<usize>>;
    fn seek(&mut self, from: SeekFrom) -> VfsResult<u64>;
    fn position(&self) -> u64;
    fn close(self);
}

pub(crate) struct DirectoryEntry {
    pub(crate) name: String,
    pub(crate) node: Node,
}

#[derive(Clone)]
pub(crate) struct Vfs {
    root: Node,
}

pub(crate) trait VfsApi: Sized {
    fn new(root: Node) -> Self;
    fn root(&self) -> Node;
    fn open<'a>(
        &'a self,
        path: &'a str,
        mode: OpenMode,
        credentials: Credentials,
    ) -> VfsFuture<'a, VfsResult<FileHandle>>;
    fn open_blocking(
        &self,
        path: &str,
        mode: OpenMode,
        credentials: Credentials,
    ) -> VfsResult<FileHandle>;
    fn resolve<'a>(
        &'a self,
        path: &'a str,
        credentials: &'a Credentials,
    ) -> VfsFuture<'a, VfsResult<Node>>;
    fn read_dir<'a>(
        &'a self,
        path: &'a str,
        credentials: &'a Credentials,
    ) -> VfsFuture<'a, VfsResult<Vec<DirectoryEntry>>>;
}
