/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::string::String;
use alloc::task::Wake;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use crate::kernel::arch::traits::CpuArchitecture as _;
use crate::kernel::error::Error;
use crate::kernel::process::{Credentials, GroupId, UserId};
use crate::kernel::scheduler::thread::{ThreadManager, ThreadState};
use crate::kernel::scheduler::sync::IrqSpinLock;

pub(crate) mod plugin;

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
const SUPPORTED_INODE_FLAGS: u32 = INODE_FLAG_IMMUTABLE
    | INODE_FLAG_APPEND
    | INODE_FLAG_NODUMP
    | INODE_FLAG_NOATIME
    | INODE_FLAG_DIRSYNC
    | INODE_FLAG_TOPDIR;
const XATTR_CREATE: i32 = 1;
const XATTR_REPLACE: i32 = 2;
const XATTR_NAME_MAX: usize = 255;
const XATTR_SIZE_MAX: usize = 64 * 1024;
const PERMISSION_MASK: u16 = 0o777;
const ACCESS_EXECUTE: u16 = 0o1;
const ACCESS_WRITE: u16 = 0o2;
const ACCESS_READ: u16 = 0o4;

static MOUNTED_VFS: IrqSpinLock<Option<Vfs>> = IrqSpinLock::new(None);

#[derive(Clone)]
pub(crate) struct NodeMetadata {
    state: Arc<IrqSpinLock<NodeMetadataState>>,
}

struct NodeMetadataState {
    owner: UserId,
    group: GroupId,
    mode: u16,
    flags: u32,
    xattrs: Vec<Xattr>,
}

struct Xattr {
    name: Vec<u8>,
    value: Vec<u8>,
}

impl NodeMetadata {
    pub(crate) fn new() -> Self {
        Self::new_with_defaults(UserId::ROOT, GroupId::ROOT, 0o644)
    }

    fn new_directory() -> Self {
        Self::new_with_defaults(UserId::ROOT, GroupId::ROOT, 0o755)
    }

    pub(crate) fn with_owner_and_mode(
        owner: UserId,
        group: GroupId,
        mode: u16,
    ) -> VfsResult<Self> {
        if mode & !PERMISSION_MASK != 0 {
            return Err(Error::Inval);
        }
        Ok(Self::new_with_defaults(owner, group, mode))
    }

    fn new_with_defaults(owner: UserId, group: GroupId, mode: u16) -> Self {
        Self {
            state: Arc::new(IrqSpinLock::new(NodeMetadataState {
                owner,
                group,
                mode,
                flags: 0,
                xattrs: Vec::new(),
            })),
        }
    }

    pub(crate) fn owner(&self) -> UserId {
        self.state.lock().owner
    }

    pub(crate) fn group(&self) -> GroupId {
        self.state.lock().group
    }

    pub(crate) fn mode(&self) -> u16 {
        self.state.lock().mode
    }

    pub(crate) fn check_access(
        &self,
        credentials: &Credentials,
        requested: u16,
        is_directory: bool,
    ) -> VfsResult<()> {
        if requested == 0 || requested & !0o7 != 0 {
            return Err(Error::Inval);
        }

        let state = self.state.lock();
        if credentials.is_root() {
            if !is_directory
                && requested & ACCESS_EXECUTE != 0
                && state.mode & 0o111 == 0
            {
                return Err(Error::Acces);
            }
            return Ok(());
        }

        let shift = if credentials.effective_user() == state.owner {
            6
        } else if credentials.is_member_of(state.group) {
            3
        } else {
            0
        };
        if (state.mode >> shift) & requested == requested {
            Ok(())
        } else {
            Err(Error::Acces)
        }
    }

    pub(crate) fn set_mode(&self, credentials: &Credentials, mode: u16) -> VfsResult<()> {
        if mode & !PERMISSION_MASK != 0 {
            return Err(Error::Inval);
        }
        let mut state = self.state.lock();
        if !credentials.is_root() && credentials.effective_user() != state.owner {
            return Err(Error::Perm);
        }
        state.mode = mode;
        Ok(())
    }

    pub(crate) fn set_owner_and_group(
        &self,
        credentials: &Credentials,
        owner: UserId,
        group: GroupId,
    ) -> VfsResult<()> {
        if !credentials.is_root() {
            return Err(Error::Perm);
        }
        let mut state = self.state.lock();
        state.owner = owner;
        state.group = group;
        Ok(())
    }

    pub(crate) fn update_from_filesystem(
        &self,
        owner: UserId,
        group: GroupId,
        mode: u16,
    ) -> VfsResult<()> {
        if mode & !PERMISSION_MASK != 0 {
            return Err(Error::Inval);
        }
        let mut state = self.state.lock();
        state.owner = owner;
        state.group = group;
        state.mode = mode;
        Ok(())
    }

    pub(crate) fn flags(&self) -> u32 {
        self.state.lock().flags
    }

    pub(crate) fn set_flags(&self, credentials: &Credentials, flags: u32) -> VfsResult<()> {
        if !credentials.is_root() {
            return Err(Error::Perm);
        }
        if flags & !SUPPORTED_INODE_FLAGS != 0 {
            return Err(Error::Inval);
        }
        self.state.lock().flags = flags;
        Ok(())
    }

    pub(crate) fn get_xattr(&self, name: &[u8]) -> VfsResult<Vec<u8>> {
        validate_xattr_name(name)?;
        let state = self.state.lock();
        let value = state
            .xattrs
            .iter()
            .find(|attribute| attribute.name == name)
            .map(|attribute| attribute.value.as_slice())
            .ok_or(Error::NoData)?;
        let mut copy = Vec::new();
        copy.try_reserve_exact(value.len()).map_err(|_| Error::NoMem)?;
        copy.extend_from_slice(value);
        Ok(copy)
    }

    pub(crate) fn list_xattrs(&self) -> VfsResult<Vec<Vec<u8>>> {
        let state = self.state.lock();
        let mut names = Vec::new();
        names
            .try_reserve_exact(state.xattrs.len())
            .map_err(|_| Error::NoMem)?;
        for attribute in &state.xattrs {
            let mut name = Vec::new();
            name.try_reserve_exact(attribute.name.len())
                .map_err(|_| Error::NoMem)?;
            name.extend_from_slice(&attribute.name);
            names.push(name);
        }
        Ok(names)
    }

    pub(crate) fn set_xattr(
        &self,
        credentials: &Credentials,
        name: &[u8],
        value: &[u8],
        flags: i32,
    ) -> VfsResult<()> {
        validate_xattr_name(name)?;
        if value.len() > XATTR_SIZE_MAX {
            return Err(Error::TooBig);
        }
        if flags & !(XATTR_CREATE | XATTR_REPLACE) != 0
            || flags == (XATTR_CREATE | XATTR_REPLACE)
        {
            return Err(Error::Inval);
        }

        let mut owned_name = Vec::new();
        owned_name
            .try_reserve_exact(name.len())
            .map_err(|_| Error::NoMem)?;
        owned_name.extend_from_slice(name);
        let mut owned_value = Vec::new();
        owned_value
            .try_reserve_exact(value.len())
            .map_err(|_| Error::NoMem)?;
        owned_value.extend_from_slice(value);

        let mut state = self.state.lock();
        if !credentials.is_root() && credentials.effective_user() != state.owner {
            return Err(Error::Perm);
        }
        if name.starts_with(b"trusted.") && !credentials.is_root() {
            return Err(Error::Perm);
        }
        if state.flags & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND) != 0 {
            return Err(Error::Perm);
        }
        if let Some(index) = state
            .xattrs
            .iter()
            .position(|attribute| attribute.name == name)
        {
            if flags & XATTR_CREATE != 0 {
                return Err(Error::Exist);
            }
            state.xattrs[index].value = owned_value;
            return Ok(());
        }
        if flags & XATTR_REPLACE != 0 {
            return Err(Error::NoData);
        }
        state.xattrs.try_reserve(1).map_err(|_| Error::NoMem)?;
        state.xattrs.push(Xattr {
            name: owned_name,
            value: owned_value,
        });
        Ok(())
    }

    pub(crate) fn remove_xattr(
        &self,
        credentials: &Credentials,
        name: &[u8],
    ) -> VfsResult<()> {
        validate_xattr_name(name)?;
        let mut state = self.state.lock();
        if !credentials.is_root() && credentials.effective_user() != state.owner {
            return Err(Error::Perm);
        }
        if name.starts_with(b"trusted.") && !credentials.is_root() {
            return Err(Error::Perm);
        }
        if state.flags & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND) != 0 {
            return Err(Error::Perm);
        }
        let index = state
            .xattrs
            .iter()
            .position(|attribute| attribute.name == name)
            .ok_or(Error::NoData)?;
        state.xattrs.remove(index);
        Ok(())
    }
}

fn validate_xattr_name(name: &[u8]) -> VfsResult<()> {
    if name.is_empty() || name.len() > XATTR_NAME_MAX || name.contains(&0) {
        Err(Error::Inval)
    } else {
        Ok(())
    }
}

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

    fn access_mask(self) -> u16 {
        match self {
            Self::ReadOnly => ACCESS_READ,
            Self::WriteOnly => ACCESS_WRITE,
            Self::ReadWrite => ACCESS_READ | ACCESS_WRITE,
        }
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
    fn open(
        &self,
        _mode: OpenMode,
    ) -> VfsFuture<'_, VfsResult<Option<Arc<dyn FilePlugin>>>> {
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

impl Node {
    pub(crate) fn directory<P: DirectoryPlugin + 'static>(plugin: P) -> Self {
        Self::directory_with_metadata(plugin, NodeMetadata::new_directory())
    }

    pub(crate) fn file<P: FilePlugin + 'static>(plugin: P) -> Self {
        Self::file_with_metadata(plugin, NodeMetadata::new())
    }

    pub(crate) fn directory_with_metadata<P: DirectoryPlugin + 'static>(
        plugin: P,
        metadata: NodeMetadata,
    ) -> Self {
        Self::Directory {
            plugin: Arc::new(plugin),
            metadata,
        }
    }

    pub(crate) fn file_with_metadata<P: FilePlugin + 'static>(
        plugin: P,
        metadata: NodeMetadata,
    ) -> Self {
        Self::File {
            plugin: Arc::new(plugin),
            metadata,
        }
    }

    pub(crate) fn metadata(&self) -> NodeMetadata {
        match self {
            Self::Directory { metadata, .. } | Self::File { metadata, .. } => metadata.clone(),
        }
    }

    pub(crate) fn flags(&self) -> u32 {
        self.metadata().flags()
    }

    fn check_access(&self, credentials: &Credentials, requested: u16) -> VfsResult<()> {
        match self {
            Self::Directory { metadata, .. } => metadata.check_access(credentials, requested, true),
            Self::File { metadata, .. } => metadata.check_access(credentials, requested, false),
        }
    }

    pub(crate) fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, VfsResult<Node>> {
        match self {
            Self::Directory { plugin, .. } => plugin.lookup(name),
            Self::File { .. } => Box::pin(async { Err(Error::NotDir) }),
        }
    }

    pub(crate) fn read_dir(&self) -> VfsFuture<'_, VfsResult<Vec<DirectoryEntry>>> {
        match self {
            Self::Directory { plugin, .. } => plugin.read_dir(),
            Self::File { .. } => Box::pin(async { Err(Error::NotDir) }),
        }
    }

    pub(crate) fn read_at<'a>(
        &'a self,
        offset: u64,
        buffer: &'a mut [u8],
    ) -> VfsFuture<'a, VfsResult<usize>> {
        match self {
            Self::File { plugin, .. } => plugin.read_at(offset, buffer),
            Self::Directory { .. } => Box::pin(async { Err(Error::IsDir) }),
        }
    }

    pub(crate) fn size(&self) -> VfsFuture<'_, VfsResult<u64>> {
        match self {
            Self::File { plugin, .. } => plugin.size(),
            Self::Directory { .. } => Box::pin(async { Err(Error::IsDir) }),
        }
    }

    pub(crate) fn write_at<'a>(
        &'a self,
        offset: u64,
        buffer: &'a [u8],
    ) -> VfsFuture<'a, VfsResult<usize>> {
        match self {
            Self::File { plugin, metadata } => {
                let plugin = Arc::clone(plugin);
                let metadata = metadata.clone();
                Box::pin(async move {
                    if metadata.flags() & INODE_FLAG_IMMUTABLE != 0 {
                        return Err(Error::Error);
                    }
                    plugin.write_at(offset, buffer).await
                })
            }
            Self::Directory { .. } => Box::pin(async { Err(Error::IsDir) }),
        }
    }

    pub(crate) fn append<'a>(
        &'a self,
        opened_plugin: Option<&'a Arc<dyn FilePlugin>>,
        buffer: &'a [u8],
    ) -> VfsFuture<'a, VfsResult<AppendResult>> {
        match self {
            Self::File { plugin, metadata } => {
                let plugin = opened_plugin.cloned().unwrap_or_else(|| Arc::clone(plugin));
                let metadata = metadata.clone();
                Box::pin(async move {
                    if metadata.flags() & INODE_FLAG_IMMUTABLE != 0 {
                        return Err(Error::Error);
                    }
                    plugin.append(buffer).await
                })
            }
            Self::Directory { .. } => Box::pin(async { Err(Error::IsDir) }),
        }
    }
}

pub(crate) struct FileHandle {
    node: Node,
    opened_plugin: Option<Arc<dyn FilePlugin>>,
    mode: OpenMode,
    offset: u64,
}

impl FileHandle {
    pub(crate) fn read_blocking(&mut self, buffer: &mut [u8]) -> VfsResult<usize> {
        run_vfs_future(self.read(buffer))
    }

    pub(crate) fn read<'a>(&'a mut self, buffer: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move {
            if !self.mode.allows_read() {
                return Err(Error::Badf);
            }
            let requested = u64::try_from(buffer.len()).map_err(|_| Error::Fbig)?;
            self.offset.checked_add(requested).ok_or(Error::Fbig)?;
            let count = match &self.opened_plugin {
                Some(plugin) => plugin.read_at(self.offset, buffer).await?,
                None => self.node.read_at(self.offset, buffer).await?,
            };
            self.offset = self
                .offset
                .checked_add(u64::try_from(count).map_err(|_| Error::Fbig)?)
                .ok_or(Error::Fbig)?;
            Ok(count)
        })
    }

    pub(crate) fn write_blocking(&mut self, buffer: &[u8]) -> VfsResult<usize> {
        run_vfs_future(self.write(buffer))
    }

    pub(crate) fn write<'a>(&'a mut self, buffer: &'a [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move {
            if !self.mode.allows_write() {
                return Err(Error::Badf);
            }
            let requested = u64::try_from(buffer.len()).map_err(|_| Error::Fbig)?;
            let append = self.node.flags() & INODE_FLAG_APPEND != 0;
            if !append {
                self.offset.checked_add(requested).ok_or(Error::Fbig)?;
            }
            if requested == 0 {
                return Ok(0);
            }
            if append {
                let result = self.node.append(self.opened_plugin.as_ref(), buffer).await?;
                if result.written > buffer.len() {
                    return Err(Error::Proto);
                }
                self.offset = result.end_offset;
                return Ok(result.written);
            }
            let count = match &self.opened_plugin {
                Some(plugin) => plugin.write_at(self.offset, buffer).await?,
                None => self.node.write_at(self.offset, buffer).await?,
            };
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
            SeekFrom::Current(delta) if delta >= 0 => {
                self.offset.checked_add(delta as u64).ok_or(Error::Inval)?
            }
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
        credentials: Credentials,
    ) -> VfsFuture<'a, VfsResult<FileHandle>> {
        Box::pin(async move {
            let node = self.resolve(path, &credentials).await?;
            let opened_plugin = match &node {
                Node::File { plugin, .. } => {
                    node.check_access(&credentials, mode.access_mask())?;
                    plugin.open(mode).await?
                }
                Node::Directory { .. } => return Err(Error::IsDir),
            };
            Ok(FileHandle {
                node,
                opened_plugin,
                mode,
                offset: 0,
            })
        })
    }

    pub(crate) fn open_blocking(
        &self,
        path: &str,
        mode: OpenMode,
        credentials: Credentials,
    ) -> VfsResult<FileHandle> {
        run_vfs_future(self.open(path, mode, credentials))
    }

    pub(crate) fn resolve<'a>(
        &'a self,
        path: &'a str,
        credentials: &'a Credentials,
    ) -> VfsFuture<'a, VfsResult<Node>> {
        Box::pin(async move {
            let mut node = self.root.clone();
            for component in path.split('/') {
                match component {
                    "" | "." => continue,
                    ".." => return Err(Error::Inval),
                    name => {
                        if matches!(node, Node::Directory { .. }) {
                            node.check_access(credentials, ACCESS_EXECUTE)?;
                        }
                        node = node.lookup(name).await?;
                    }
                }
            }
            Ok(node)
        })
    }

    pub(crate) fn read_dir<'a>(
        &'a self,
        path: &'a str,
        credentials: &'a Credentials,
    ) -> VfsFuture<'a, VfsResult<Vec<DirectoryEntry>>> {
        Box::pin(async move {
            let node = self.resolve(path, credentials).await?;
            node.check_access(credentials, ACCESS_READ | ACCESS_EXECUTE)?;
            node.read_dir().await
        })
    }
}

pub(crate) fn mount_virtio_devices(virtio_fs_root: Option<Node>) -> usize {
    let (root, device_count) = plugin::build_virtio_tree(virtio_fs_root);
    *MOUNTED_VFS.lock() = Some(Vfs::new(root));
    device_count
}

pub(crate) fn mounted() -> Option<Vfs> {
    MOUNTED_VFS.lock().as_ref().cloned()
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn run_vfs_future<T>(mut future: VfsFuture<'_, VfsResult<T>>) -> VfsResult<T> {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(result) => return result,
            Poll::Pending => {
                if !ThreadManager::switch_current(ThreadState::Ready) {
                    crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
                }
            }
        }
    }
}
