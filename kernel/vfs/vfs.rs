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
use core::borrow::Borrow;
use core::hash::{Hash, Hasher};
use core::task::{Context, Poll, Waker};

use crate::kernel::arch::traits::CpuArchitecture as _;
use crate::kernel::error::Error;
use crate::kernel::lib::hashmap::HashMap;
use crate::kernel::lib::immutable_byte_buffer::ImmutableByteBuffer;
use crate::kernel::process::{Credentials, GroupId, UserId};
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::scheduler::thread::{ThreadManager, ThreadState};

use super::{
    AppendResult, DirectoryEntry, DirectoryPlugin, FileHandle, FileHandleApi, FilePlugin,
    INODE_FLAG_APPEND, INODE_FLAG_DIRSYNC, INODE_FLAG_IMMUTABLE, INODE_FLAG_NODUMP,
    INODE_FLAG_NOATIME, INODE_FLAG_TOPDIR, Node, NodeApi, NodeMetadata, NodeMetadataApi, OpenMode,
    SeekFrom, Vfs, VfsApi, VfsFuture, VfsResult,
};

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

#[derive(Clone)]
struct XattrName(Arc<ImmutableByteBuffer>);

impl XattrName {
    fn try_from_slice(name: &[u8]) -> VfsResult<Self> {
        Ok(Self(Arc::new(ImmutableByteBuffer::try_from_slice(name)?)))
    }

    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl Borrow<[u8]> for XattrName {
    fn borrow(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl PartialEq for XattrName {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for XattrName {}

impl Hash for XattrName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

pub(super) struct NodeMetadataState {
    owner: UserId,
    group: GroupId,
    mode: u16,
    flags: u32,
    xattrs: HashMap<XattrName, Arc<ImmutableByteBuffer>, 1>,
}

static MOUNTED_VFS: IrqSpinLock<Option<Vfs>> = IrqSpinLock::new(None);

impl NodeMetadata {
    fn new_directory() -> Self {
        Self::new_with_defaults(UserId::ROOT, GroupId::ROOT, 0o755)
    }

    fn new_with_defaults(owner: UserId, group: GroupId, mode: u16) -> Self {
        Self {
            state: Arc::new(IrqSpinLock::new(NodeMetadataState {
                owner,
                group,
                mode,
                flags: 0,
                xattrs: HashMap::new(),
            })),
        }
    }
}

impl NodeMetadataApi for NodeMetadata {
    fn new() -> Self {
        Self::new_with_defaults(UserId::ROOT, GroupId::ROOT, 0o644)
    }

    fn with_owner_and_mode(owner: UserId, group: GroupId, mode: u16) -> VfsResult<Self> {
        if mode & !PERMISSION_MASK != 0 {
            return Err(Error::Inval);
        }
        Ok(Self::new_with_defaults(owner, group, mode))
    }

    fn owner(&self) -> UserId {
        self.state.lock().owner
    }

    fn group(&self) -> GroupId {
        self.state.lock().group
    }

    fn mode(&self) -> u16 {
        self.state.lock().mode
    }

    fn check_access(
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
            if !is_directory && requested & ACCESS_EXECUTE != 0 && state.mode & 0o111 == 0 {
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

    fn set_mode(&self, credentials: &Credentials, mode: u16) -> VfsResult<()> {
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

    fn set_owner_and_group(
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

    fn update_from_filesystem(&self, owner: UserId, group: GroupId, mode: u16) -> VfsResult<()> {
        if mode & !PERMISSION_MASK != 0 {
            return Err(Error::Inval);
        }
        let mut state = self.state.lock();
        state.owner = owner;
        state.group = group;
        state.mode = mode;
        Ok(())
    }

    fn flags(&self) -> u32 {
        self.state.lock().flags
    }

    fn set_flags(&self, credentials: &Credentials, flags: u32) -> VfsResult<()> {
        if !credentials.is_root() {
            return Err(Error::Perm);
        }
        if flags & !SUPPORTED_INODE_FLAGS != 0 {
            return Err(Error::Inval);
        }
        self.state.lock().flags = flags;
        Ok(())
    }

    fn get_xattr(&self, name: &[u8]) -> VfsResult<Arc<ImmutableByteBuffer>> {
        validate_xattr_name(name)?;
        let state = self.state.lock();
        state
            .xattrs
            .get(name)
            .cloned()
            .ok_or(Error::NoData)
    }

    fn list_xattrs(&self) -> VfsResult<Vec<Arc<ImmutableByteBuffer>>> {
        let state = self.state.lock();
        let mut names = Vec::new();
        names
            .try_reserve_exact(state.xattrs.len())
            .map_err(|_| Error::NoMem)?;
        state.xattrs.try_for_each(|attribute_name, _attribute_value| {
            names.push(Arc::clone(&attribute_name.0));
            Ok::<(), Error>(())
        })?;
        Ok(names)
    }

    fn set_xattr(
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

        let owned_name = XattrName::try_from_slice(name)?;
        let owned_value = Arc::new(ImmutableByteBuffer::try_from_slice(value)?);

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
        let already_exists = state.xattrs.contains_key(name);
        if already_exists && flags & XATTR_CREATE != 0 {
            return Err(Error::Exist);
        }
        if !already_exists && flags & XATTR_REPLACE != 0 {
            return Err(Error::NoData);
        }
        let _previous = state
            .xattrs
            .insert(owned_name, owned_value)
            .map_err(|(_name, _value)| Error::NoMem)?;
        Ok(())
    }

    fn remove_xattr(&self, credentials: &Credentials, name: &[u8]) -> VfsResult<()> {
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
        state.xattrs.remove(name).ok_or(Error::NoData)?;
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

impl Node {
    fn check_access(&self, credentials: &Credentials, requested: u16) -> VfsResult<()> {
        match self {
            Self::Directory { metadata, .. } => metadata.check_access(credentials, requested, true),
            Self::File { metadata, .. } => metadata.check_access(credentials, requested, false),
        }
    }
}

impl NodeApi for Node {
    fn directory<P: DirectoryPlugin + 'static>(plugin: P) -> Self {
        Self::directory_with_metadata(plugin, NodeMetadata::new_directory())
    }

    fn file<P: FilePlugin + 'static>(plugin: P) -> Self {
        Self::file_with_metadata(plugin, NodeMetadata::new())
    }

    fn directory_with_metadata<P: DirectoryPlugin + 'static>(
        plugin: P,
        metadata: NodeMetadata,
    ) -> Self {
        Self::Directory {
            plugin: Arc::new(plugin),
            metadata,
        }
    }

    fn file_with_metadata<P: FilePlugin + 'static>(plugin: P, metadata: NodeMetadata) -> Self {
        Self::File {
            plugin: Arc::new(plugin),
            metadata,
        }
    }

    fn metadata(&self) -> NodeMetadata {
        match self {
            Self::Directory { metadata, .. } | Self::File { metadata, .. } => metadata.clone(),
        }
    }

    fn flags(&self) -> u32 {
        self.metadata().flags()
    }

    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, VfsResult<Node>> {
        match self {
            Self::Directory { plugin, .. } => plugin.lookup(name),
            Self::File { .. } => Box::pin(async { Err(Error::NotDir) }),
        }
    }

    fn read_dir(&self) -> VfsFuture<'_, VfsResult<Vec<DirectoryEntry>>> {
        match self {
            Self::Directory { plugin, .. } => plugin.read_dir(),
            Self::File { .. } => Box::pin(async { Err(Error::NotDir) }),
        }
    }

    fn read_at<'a>(
        &'a self,
        offset: u64,
        buffer: &'a mut [u8],
    ) -> VfsFuture<'a, VfsResult<usize>> {
        match self {
            Self::File { plugin, .. } => plugin.read_at(offset, buffer),
            Self::Directory { .. } => Box::pin(async { Err(Error::IsDir) }),
        }
    }

    fn size(&self) -> VfsFuture<'_, VfsResult<u64>> {
        match self {
            Self::File { plugin, .. } => plugin.size(),
            Self::Directory { .. } => Box::pin(async { Err(Error::IsDir) }),
        }
    }

    fn write_at<'a>(
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

    fn append<'a>(
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

impl FileHandleApi for FileHandle {
    fn read_blocking(&mut self, buffer: &mut [u8]) -> VfsResult<usize> {
        run_vfs_future(self.read(buffer))
    }

    fn read<'a>(&'a mut self, buffer: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>> {
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

    fn write_blocking(&mut self, buffer: &[u8]) -> VfsResult<usize> {
        run_vfs_future(self.write(buffer))
    }

    fn write<'a>(&'a mut self, buffer: &'a [u8]) -> VfsFuture<'a, VfsResult<usize>> {
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

    fn seek(&mut self, from: SeekFrom) -> VfsResult<u64> {
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

    fn position(&self) -> u64 {
        self.offset
    }

    fn close(self) {
        drop(self);
    }
}

impl VfsApi for Vfs {
    fn new(root: Node) -> Self {
        Self { root }
    }

    fn root(&self) -> Node {
        self.root.clone()
    }

    fn open<'a>(
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

    fn open_blocking(
        &self,
        path: &str,
        mode: OpenMode,
        credentials: Credentials,
    ) -> VfsResult<FileHandle> {
        run_vfs_future(self.open(path, mode, credentials))
    }

    fn resolve<'a>(
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

    fn read_dir<'a>(
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
    let (root, device_count) = super::plugin::build_virtio_tree(virtio_fs_root);
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

#[cfg(test)]
#[path = "../../tests/kernel/vfs/vfs.rs"]
mod tests;
