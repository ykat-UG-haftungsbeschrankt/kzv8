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
use core::borrow::Borrow;
use core::hash::{Hash, Hasher};
use core::task::{Context, Poll, Waker};

use crate::kernel::arch::traits::CpuArchitecture as _;
use crate::kernel::error::Error;
use crate::kernel::lib::hashmap::HashMap;
use crate::kernel::lib::immutable_byte_buffer::ImmutableByteBuffer;
use crate::kernel::credentials::{GroupId, UserId};
use crate::kernel::process::Credentials;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::scheduler::thread::{ThreadManager, ThreadState};

use super::{
    AppendResult, DirectoryEntry, DirectoryPlugin, FileHandle, FileHandleApi, FilePlugin,
    INODE_FLAG_APPEND, INODE_FLAG_DIRSYNC, INODE_FLAG_IMMUTABLE, INODE_FLAG_NODUMP,
    INODE_FLAG_NOATIME, INODE_FLAG_TOPDIR, Node, NodeApi, NodeMetadata, NodeMetadataApi, OpenMode,
    SeekFrom, VfsApi, VfsFuture, VfsResult,
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
    link_count: u32,
    flags: u32,
    xattrs: HashMap<XattrName, Arc<ImmutableByteBuffer>, 1>,
}

static MOUNTED_VFS: IrqSpinLock<Option<Vfs>> = IrqSpinLock::new(None);

const MAX_SYMLINK_FOLLOWS: usize = 40;

#[derive(Clone)]
pub(crate) struct Vfs {
    root: Node,
    mounts: Arc<IrqSpinLock<Vec<MountEntry>>>,
}

struct MountEntry {
    path: String,
    root: Node,
}

enum ResolveStep {
    Restart(String),
    Resolved(Node, String),
}

impl Vfs {
    fn mount_root_at(&self, path: &str) -> Option<Node> {
        self.mounts
            .lock()
            .iter()
            .find(|mount| mount.path == path)
            .map(|mount| mount.root.clone())
    }

    fn mount_path_for<'a>(mounts: &'a [MountEntry], path: &str) -> Option<&'a str> {
        mounts
            .iter()
            .filter(|mount| path_is_within(path, &mount.path))
            .max_by_key(|mount| mount.path.len())
            .map(|mount| mount.path.as_str())
    }

    async fn resolve_internal(
        &self,
        path: &str,
        credentials: &Credentials,
        follow_final_symlink: bool,
    ) -> VfsResult<(Node, String)> {
        if path.contains('\0') {
            return Err(Error::Inval);
        }
        let mut pending_path = copy_string(path)?;
        let mut followed_symlinks = 0;
        loop {
            let step = {
                let mut components = Vec::new();
                components
                    .try_reserve_exact(pending_path.split('/').count())
                    .map_err(|_| Error::NoMem)?;
                components.extend(pending_path.split('/').filter(|part| !part.is_empty()));

                let mut node_stack = Vec::new();
                node_stack
                    .try_reserve_exact(components.len().saturating_add(1))
                    .map_err(|_| Error::NoMem)?;
                node_stack.push(self.root.clone());

                let mut path_stack: Vec<&str> = Vec::new();
                path_stack
                    .try_reserve_exact(components.len())
                    .map_err(|_| Error::NoMem)?;

                let mut restart = None;
                for (index, component) in components.iter().enumerate() {
                    let component = *component;
                    match component {
                        "." | ".." => {
                            let current = node_stack.last().ok_or(Error::Proto)?;
                            if !matches!(current, Node::Directory { .. }) {
                                return Err(Error::NotDir);
                            }
                            current.check_access(credentials, ACCESS_EXECUTE)?;
                            if component == ".." && node_stack.len() > 1 {
                                node_stack.pop();
                                path_stack.pop();
                            }
                            continue;
                        }
                        _ => {}
                    }

                    let parent = node_stack.last().cloned().ok_or(Error::Proto)?;
                    if !matches!(&parent, Node::Directory { .. }) {
                        return Err(Error::NotDir);
                    }
                    parent.check_access(credentials, ACCESS_EXECUTE)?;
                    let mut child = parent.lookup(component).await?;
                    let child_path = absolute_path_with(&path_stack, component)?;
                    if let Some(mounted_root) = self.mount_root_at(&child_path) {
                        child = mounted_root;
                    }

                    let is_final = index + 1 == components.len();
                    if let Node::Symlink { target, .. } = &child {
                        if !is_final || follow_final_symlink || pending_path.ends_with('/') {
                            followed_symlinks += 1;
                            if followed_symlinks > MAX_SYMLINK_FOLLOWS {
                                return Err(Error::Loop);
                            }
                            restart = Some(expand_symlink_path(
                                &path_stack,
                                target,
                                &components[index + 1..],
                                pending_path.ends_with('/'),
                            )?);
                            break;
                        }
                    }

                    path_stack.push(component);
                    node_stack.push(child);
                }

                match restart {
                    Some(path) => ResolveStep::Restart(path),
                    None => {
                        let node = node_stack.pop().ok_or(Error::Proto)?;
                        if pending_path.ends_with('/')
                            && !matches!(&node, Node::Directory { .. })
                        {
                            return Err(Error::NotDir);
                        }
                        ResolveStep::Resolved(node, absolute_path(&path_stack)?)
                    }
                }
            };

            match step {
                ResolveStep::Restart(path) => pending_path = path,
                ResolveStep::Resolved(node, path) => return Ok((node, path)),
            }
        }
    }

    fn same_filesystem(&self, first: &str, second: &str) -> bool {
        let mounts = self.mounts.lock();
        Self::mount_path_for(&mounts, first) == Self::mount_path_for(&mounts, second)
    }

    fn is_mountpoint(&self, path: &str) -> bool {
        self.mounts.lock().iter().any(|mount| mount.path == path)
    }
}

fn copy_string(value: &str) -> VfsResult<String> {
    let mut owned = String::new();
    owned
        .try_reserve_exact(value.len())
        .map_err(|_| Error::NoMem)?;
    owned.push_str(value);
    Ok(owned)
}

fn absolute_path(components: &[&str]) -> VfsResult<String> {
    let mut path = String::new();
    path.try_reserve_exact(1)
        .map_err(|_| Error::NoMem)?;
    path.push('/');
    for component in components {
        push_path_component(&mut path, component)?;
    }
    Ok(path)
}

fn absolute_path_with(components: &[&str], component: &str) -> VfsResult<String> {
    let mut path = absolute_path(components)?;
    push_path_component(&mut path, component)?;
    Ok(path)
}

fn push_path_component(path: &mut String, component: &str) -> VfsResult<()> {
    if path.len() > 1 {
        path.try_reserve(1).map_err(|_| Error::NoMem)?;
        path.push('/');
    }
    path.try_reserve(component.len()).map_err(|_| Error::NoMem)?;
    path.push_str(component);
    Ok(())
}

fn expand_symlink_path(
    parent: &[&str],
    target: &str,
    remaining: &[&str],
    trailing_slash: bool,
) -> VfsResult<String> {
    let mut expanded = if target.starts_with('/') {
        copy_string(target)?
    } else {
        let mut path = absolute_path(parent)?;
        if path != "/" {
            path.try_reserve(1).map_err(|_| Error::NoMem)?;
            path.push('/');
        }
        path.try_reserve(target.len()).map_err(|_| Error::NoMem)?;
        path.push_str(target);
        path
    };
    for component in remaining {
        if !expanded.ends_with('/') {
            expanded.try_reserve(1).map_err(|_| Error::NoMem)?;
            expanded.push('/');
        }
        expanded
            .try_reserve(component.len())
            .map_err(|_| Error::NoMem)?;
        expanded.push_str(component);
    }
    if trailing_slash && !expanded.ends_with('/') {
        expanded.try_reserve(1).map_err(|_| Error::NoMem)?;
        expanded.push('/');
    }
    Ok(expanded)
}

fn path_is_within(path: &str, mount_path: &str) -> bool {
    path == mount_path
        || (path.starts_with(mount_path)
            && path.as_bytes().get(mount_path.len()) == Some(&b'/'))
}

fn split_parent_path(path: &str) -> VfsResult<(&str, &str)> {
    if path.contains('\0') {
        return Err(Error::Inval);
    }
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return Err(Error::Busy);
    }
    let Some(separator) = path.rfind('/') else {
        return validate_entry_name(path).map(|()| ("", path));
    };
    let parent = if separator == 0 {
        "/"
    } else {
        &path[..separator]
    };
    let name = &path[separator + 1..];
    validate_entry_name(name)?;
    Ok((parent, name))
}

fn validate_entry_name(name: &str) -> VfsResult<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        Err(Error::Inval)
    } else {
        Ok(())
    }
}

fn child_absolute_path(parent: &str, name: &str) -> VfsResult<String> {
    let mut path = copy_string(parent)?;
    if path != "/" {
        path.try_reserve(1).map_err(|_| Error::NoMem)?;
        path.push('/');
    }
    path.try_reserve(name.len()).map_err(|_| Error::NoMem)?;
    path.push_str(name);
    Ok(path)
}

fn check_directory_mutation(
    node: &Node,
    credentials: &Credentials,
    removes_entry: bool,
) -> VfsResult<()> {
    let Node::Directory { metadata, .. } = node else {
        return Err(Error::NotDir);
    };
    metadata.check_access(credentials, ACCESS_WRITE | ACCESS_EXECUTE, true)?;
    let flags = metadata.flags();
    if flags & INODE_FLAG_IMMUTABLE != 0 || removes_entry && flags & INODE_FLAG_APPEND != 0 {
        return Err(Error::Perm);
    }
    Ok(())
}

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
                link_count: 1,
                flags: 0,
                xattrs: HashMap::new(),
            })),
        }
    }

    fn add_link(&self) -> VfsResult<()> {
        let mut state = self.state.lock();
        state.link_count = state.link_count.checked_add(1).ok_or(Error::Mlink)?;
        Ok(())
    }

    fn remove_link(&self) -> VfsResult<()> {
        let mut state = self.state.lock();
        state.link_count = state.link_count.checked_sub(1).ok_or(Error::Proto)?;
        Ok(())
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

    fn link_count(&self) -> u32 {
        self.state.lock().link_count
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
            Self::Symlink { .. } => Err(Error::Inval),
        }
    }

    fn insert_entry(&self, name: &str, node: Node) -> VfsResult<()> {
        match self {
            Self::Directory { plugin, .. } => plugin.insert_entry(name, node),
            Self::File { .. } | Self::Symlink { .. } => Err(Error::NotDir),
        }
    }

    fn remove_file_entry(&self, name: &str) -> VfsResult<Node> {
        match self {
            Self::Directory { plugin, .. } => plugin.remove_file_entry(name),
            Self::File { .. } | Self::Symlink { .. } => Err(Error::NotDir),
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
            Self::Directory { metadata, .. }
            | Self::File { metadata, .. }
            | Self::Symlink { metadata, .. } => metadata.clone(),
        }
    }

    fn flags(&self) -> u32 {
        self.metadata().flags()
    }

    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, VfsResult<Node>> {
        match self {
            Self::Directory { plugin, .. } => plugin.lookup(name),
            Self::File { .. } | Self::Symlink { .. } => Box::pin(async { Err(Error::NotDir) }),
        }
    }

    fn read_dir(&self) -> VfsFuture<'_, VfsResult<Vec<DirectoryEntry>>> {
        match self {
            Self::Directory { plugin, .. } => plugin.read_dir(),
            Self::File { .. } | Self::Symlink { .. } => Box::pin(async { Err(Error::NotDir) }),
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
            Self::Symlink { .. } => Box::pin(async { Err(Error::Inval) }),
        }
    }

    fn size(&self) -> VfsFuture<'_, VfsResult<u64>> {
        match self {
            Self::File { plugin, .. } => plugin.size(),
            Self::Directory { .. } => Box::pin(async { Err(Error::IsDir) }),
            Self::Symlink { .. } => Box::pin(async { Err(Error::Inval) }),
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
            Self::Symlink { .. } => Box::pin(async { Err(Error::Inval) }),
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
            Self::Symlink { .. } => Box::pin(async { Err(Error::Inval) }),
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
        Self {
            root,
            mounts: Arc::new(IrqSpinLock::new(Vec::new())),
        }
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
                Node::Symlink { .. } => return Err(Error::Inval),
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
            self.resolve_internal(path, credentials, true)
                .await
                .map(|(node, _path)| node)
        })
    }

    fn hard_link<'a>(
        &'a self,
        source_path: &'a str,
        link_path: &'a str,
        credentials: &'a Credentials,
    ) -> VfsFuture<'a, VfsResult<()>> {
        Box::pin(async move {
            let (source, source_canonical_path) = self
                .resolve_internal(source_path, credentials, false)
                .await?;
            if matches!(&source, Node::Directory { .. }) {
                return Err(Error::Perm);
            }
            if source.flags() & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND) != 0 {
                return Err(Error::Perm);
            }
            let (parent_path, name) = split_parent_path(link_path)?;
            let (parent, parent_canonical_path) = self
                .resolve_internal(parent_path, credentials, true)
                .await?;
            check_directory_mutation(&parent, credentials, false)?;
            if !self.same_filesystem(&source_canonical_path, &parent_canonical_path) {
                return Err(Error::Xdev);
            }

            let metadata = source.metadata();
            metadata.add_link()?;
            if let Err(error) = parent.insert_entry(name, source) {
                let _rollback = metadata.remove_link();
                return Err(error);
            }
            Ok(())
        })
    }

    fn symlink<'a>(
        &'a self,
        target: &'a str,
        link_path: &'a str,
        credentials: &'a Credentials,
    ) -> VfsFuture<'a, VfsResult<()>> {
        Box::pin(async move {
            if target.is_empty() {
                return Err(Error::NoEnt);
            }
            if target.contains('\0') {
                return Err(Error::Inval);
            }
            let (parent_path, name) = split_parent_path(link_path)?;
            let (parent, _) = self
                .resolve_internal(parent_path, credentials, true)
                .await?;
            check_directory_mutation(&parent, credentials, false)?;

            let metadata = NodeMetadata::with_owner_and_mode(
                credentials.effective_user(),
                credentials.effective_group(),
                0o777,
            )?;
            let node = Node::Symlink {
                target: Arc::new(copy_string(target)?),
                metadata,
            };
            parent.insert_entry(name, node)
        })
    }

    fn read_link<'a>(
        &'a self,
        path: &'a str,
        credentials: &'a Credentials,
    ) -> VfsFuture<'a, VfsResult<String>> {
        Box::pin(async move {
            let (node, _) = self.resolve_internal(path, credentials, false).await?;
            match node {
                Node::Symlink { target, .. } => copy_string(target.as_str()),
                _ => Err(Error::Inval),
            }
        })
    }

    fn unlink<'a>(
        &'a self,
        path: &'a str,
        credentials: &'a Credentials,
    ) -> VfsFuture<'a, VfsResult<()>> {
        Box::pin(async move {
            let (parent_path, name) = split_parent_path(path)?;
            let (parent, parent_canonical_path) = self
                .resolve_internal(parent_path, credentials, true)
                .await?;
            check_directory_mutation(&parent, credentials, true)?;
            let link_path = child_absolute_path(&parent_canonical_path, name)?;
            if self.is_mountpoint(&link_path) {
                return Err(Error::Busy);
            }
            let existing = parent.lookup(name).await?;
            if existing.flags() & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND) != 0 {
                return Err(Error::Perm);
            }
            let node = parent.remove_file_entry(name)?;
            node.metadata().remove_link()
        })
    }

    fn mount<'a>(
        &'a self,
        path: &'a str,
        filesystem_root: Node,
        credentials: &'a Credentials,
    ) -> VfsFuture<'a, VfsResult<()>> {
        Box::pin(async move {
            if !credentials.is_root() {
                return Err(Error::Perm);
            }
            if !matches!(&filesystem_root, Node::Directory { .. }) {
                return Err(Error::NotDir);
            }
            let (mountpoint, mount_path) = self.resolve_internal(path, credentials, true).await?;
            if !matches!(&mountpoint, Node::Directory { .. }) {
                return Err(Error::NotDir);
            }
            if mount_path == "/" {
                return Err(Error::Busy);
            }

            let mut mounts = self.mounts.lock();
            if mounts.iter().any(|mount| {
                mount.path == mount_path || path_is_within(&mount.path, &mount_path)
            }) {
                return Err(Error::Busy);
            }
            mounts.try_reserve(1).map_err(|_| Error::NoMem)?;
            mounts.push(MountEntry {
                path: mount_path,
                root: filesystem_root,
            });
            Ok(())
        })
    }

    fn unmount<'a>(
        &'a self,
        path: &'a str,
        credentials: &'a Credentials,
    ) -> VfsFuture<'a, VfsResult<()>> {
        Box::pin(async move {
            if !credentials.is_root() {
                return Err(Error::Perm);
            }
            let (_mountpoint, mount_path) = self.resolve_internal(path, credentials, true).await?;
            if mount_path == "/" {
                return Err(Error::Busy);
            }

            let mut mounts = self.mounts.lock();
            let index = mounts
                .iter()
                .position(|mount| mount.path == mount_path)
                .ok_or(Error::NoEnt)?;
            if mounts
                .iter()
                .any(|mount| mount.path != mount_path && path_is_within(&mount.path, &mount_path))
            {
                return Err(Error::Busy);
            }
            mounts.remove(index);
            Ok(())
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

pub(crate) fn mount_virtio_devices(virtio_fs_root: Option<Node>) -> Result<usize, Error> {
    let (root, device_count) = super::plugin::build_virtio_tree(virtio_fs_root)?;
    *MOUNTED_VFS.lock() = Some(Vfs::new(root));
    Ok(device_count)
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
