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

pub(crate) type VfsResult<T> = Result<T, Error>;
pub(crate) type VfsFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

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

pub(crate) struct DirectoryEntry {
    pub(crate) name: String,
    pub(crate) node: Node,
}

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
