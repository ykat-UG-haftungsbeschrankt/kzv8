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
use core::future::Future;
use core::pin::Pin;

use crate::kernel::error::Error;
use crate::kernel::scheduler::sync::IrqSpinLock;

pub(crate) mod plugin;

pub(crate) type VfsResult<T> = Result<T, Error>;
pub(crate) type VfsFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

static MOUNTED_VFS: IrqSpinLock<Option<Vfs>> = IrqSpinLock::new(None);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OpenMode {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

impl OpenMode {
    fn allows_read(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadWrite)
    }

    fn allows_write(self) -> bool {
        matches!(self, Self::WriteOnly | Self::ReadWrite)
    }
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
    fn read_at<'a>(&'a self, offset: u64, buffer: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>>;

    fn write_at<'a>(&'a self, _offset: u64, _buffer: &'a [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async { Err(Error::Rofs) })
    }
}

#[derive(Clone)]
pub(crate) enum Node {
    Directory(Arc<dyn DirectoryPlugin>),
    File(Arc<dyn FilePlugin>),
}

impl Node {
    pub(crate) fn directory<P: DirectoryPlugin + 'static>(plugin: P) -> Self {
        Self::Directory(Arc::new(plugin))
    }

    pub(crate) fn file<P: FilePlugin + 'static>(plugin: P) -> Self {
        Self::File(Arc::new(plugin))
    }

    pub(crate) fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, VfsResult<Node>> {
        match self {
            Self::Directory(plugin) => plugin.lookup(name),
            Self::File(_) => Box::pin(async { Err(Error::NotDir) }),
        }
    }

    pub(crate) fn read_dir(&self) -> VfsFuture<'_, VfsResult<Vec<DirectoryEntry>>> {
        match self {
            Self::Directory(plugin) => plugin.read_dir(),
            Self::File(_) => Box::pin(async { Err(Error::NotDir) }),
        }
    }

    pub(crate) fn read_at<'a>(
        &'a self,
        offset: u64,
        buffer: &'a mut [u8],
    ) -> VfsFuture<'a, VfsResult<usize>> {
        match self {
            Self::File(plugin) => plugin.read_at(offset, buffer),
            Self::Directory(_) => Box::pin(async { Err(Error::IsDir) }),
        }
    }

    pub(crate) fn write_at<'a>(
        &'a self,
        offset: u64,
        buffer: &'a [u8],
    ) -> VfsFuture<'a, VfsResult<usize>> {
        match self {
            Self::File(plugin) => plugin.write_at(offset, buffer),
            Self::Directory(_) => Box::pin(async { Err(Error::IsDir) }),
        }
    }
}

pub(crate) struct FileHandle {
    node: Node,
    mode: OpenMode,
    offset: u64,
}

impl FileHandle {
    pub(crate) fn read<'a>(&'a mut self, buffer: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move {
            if !self.mode.allows_read() {
                return Err(Error::Badf);
            }
            let requested = u64::try_from(buffer.len()).map_err(|_| Error::Fbig)?;
            self.offset.checked_add(requested).ok_or(Error::Fbig)?;
            let count = self.node.read_at(self.offset, buffer).await?;
            self.offset = self
                .offset
                .checked_add(u64::try_from(count).map_err(|_| Error::Fbig)?)
                .ok_or(Error::Fbig)?;
            Ok(count)
        })
    }

    pub(crate) fn write<'a>(&'a mut self, buffer: &'a [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move {
            if !self.mode.allows_write() {
                return Err(Error::Badf);
            }
            let requested = u64::try_from(buffer.len()).map_err(|_| Error::Fbig)?;
            self.offset.checked_add(requested).ok_or(Error::Fbig)?;
            let count = self.node.write_at(self.offset, buffer).await?;
            self.offset = self
                .offset
                .checked_add(u64::try_from(count).map_err(|_| Error::Fbig)?)
                .ok_or(Error::Fbig)?;
            Ok(count)
        })
    }

    pub(crate) fn seek(&mut self, from: SeekFrom) -> VfsResult<u64> {
        let offset = match from {
            SeekFrom::Start(offset) => offset,
            SeekFrom::Current(delta) if delta >= 0 => self
                .offset
                .checked_add(delta as u64)
                .ok_or(Error::Inval)?,
            SeekFrom::Current(delta) => self
                .offset
                .checked_sub(delta.unsigned_abs())
                .ok_or(Error::Inval)?,
        };
        self.offset = offset;
        Ok(offset)
    }

    pub(crate) const fn position(&self) -> u64 {
        self.offset
    }

    pub(crate) fn close(self) {
        drop(self);
    }
}

pub(crate) struct DirectoryEntry {
    pub(crate) name: String,
    pub(crate) node: Node,
}

#[derive(Clone)]
pub(crate) struct Vfs {
    root: Node,
}

impl Vfs {
    pub(crate) fn new(root: Node) -> Self {
        Self { root }
    }

    pub(crate) fn root(&self) -> Node {
        self.root.clone()
    }

    pub(crate) fn open<'a>(
        &'a self,
        path: &'a str,
        mode: OpenMode,
    ) -> VfsFuture<'a, VfsResult<FileHandle>> {
        Box::pin(async move {
            let node = self.resolve(path).await?;
            if matches!(&node, Node::Directory(_)) {
                return Err(Error::IsDir);
            }
            Ok(FileHandle {
                node,
                mode,
                offset: 0,
            })
        })
    }

    pub(crate) fn resolve<'a>(&'a self, path: &'a str) -> VfsFuture<'a, VfsResult<Node>> {
        Box::pin(async move {
            let mut node = self.root.clone();
            for component in path.split('/') {
                match component {
                    "" | "." => continue,
                    ".." => return Err(Error::Inval),
                    name => node = node.lookup(name).await?,
                }
            }
            Ok(node)
        })
    }
}

pub(crate) fn mount_virtio_devices() -> usize {
    let (root, device_count) = plugin::build_virtio_tree();
    *MOUNTED_VFS.lock() = Some(Vfs::new(root));
    device_count
}

pub(crate) fn mounted() -> Option<Vfs> {
    MOUNTED_VFS.lock().as_ref().cloned()
}
