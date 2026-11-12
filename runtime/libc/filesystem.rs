/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::ffi::{c_char, c_void, CStr};
use core::ptr::{read_volatile, write_volatile};

use crate::kernel::arch::traits::CpuArchitecture as _;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::error::Error as KernelError;
use crate::kernel::process::{Credentials, GroupId, UserId};
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::vfs::{NodeMetadata, INODE_FLAG_APPEND, INODE_FLAG_IMMUTABLE};

use super::errno::{
    E2BIG, EAGAIN, EBADF, EBUSY, EEXIST, EFAULT, EFBIG, EINVAL, EISDIR, EMFILE, ENAMETOOLONG, ENOENT,
    ENOMEM, ENOTDIR, ENOTEMPTY, ENOTTY, EPERM, ERANGE,
};
use super::network;

const O_ACCMODE: i32 = 0o3;
const O_RDONLY: i32 = 0o0;
const O_WRONLY: i32 = 0o1;
#[allow(dead_code)]
const O_RDWR: i32 = 0o2;
const O_CREAT: i32 = 0o100;
const O_EXCL: i32 = 0o200;
const O_NOCTTY: i32 = 0o400;
const O_TRUNC: i32 = 0o1000;
const O_APPEND: i32 = 0o2000;
const O_NONBLOCK: i32 = 0o4000;
const O_LARGEFILE: i32 = 0o100000;
const O_DIRECTORY: i32 = 0o200000;
const O_NOFOLLOW: i32 = 0o400000;
const O_CLOEXEC: i32 = 0o2000000;
const O_SUPPORTED: i32 = O_ACCMODE
    | O_CREAT
    | O_EXCL
    | O_NOCTTY
    | O_TRUNC
    | O_APPEND
    | O_NONBLOCK
    | O_LARGEFILE
    | O_DIRECTORY
    | O_NOFOLLOW
    | O_CLOEXEC;

const S_IFMT: u32 = 0o170000;
const S_IFIFO: u32 = 0o010000;
const S_IFDIR: u32 = 0o040000;
const S_IFCHR: u32 = 0o020000;
const S_IFREG: u32 = 0o100000;
const S_IFSOCK: u32 = 0o140000;
const PATH_MAX: usize = 4096;
const DESCRIPTOR_COUNT: usize = 64;
const FIRST_DESCRIPTOR: i32 = 3;
const AT_FDCWD: i32 = -100;
const ROOT_NODE: usize = usize::MAX;
const TERMINAL_NODE: usize = usize::MAX - 1;
const PIPE_NODE: usize = usize::MAX - 2;
const PIPE_CAPACITY: usize = 4096;
const PIPE_BUF: usize = 512;
const XATTR_NAME_MAX: usize = 255;
const XATTR_SIZE_MAX: usize = 64 * 1024;

#[repr(C)]
#[derive(Clone, Copy)]
struct Timespec {
    seconds: i64,
    nanoseconds: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct CStat {
    device: u64,
    inode: u64,
    links: u64,
    mode: u32,
    user: u32,
    group: u32,
    pad: u32,
    special_device: u64,
    pub(crate) size: i64,
    block_size: i64,
    blocks: i64,
    accessed: Timespec,
    modified: Timespec,
    changed: Timespec,
    reserved: [i64; 3],
}

#[derive(Clone)]
struct Node {
    path: Option<Vec<u8>>,
    data: Vec<u8>,
    open_count: usize,
    inode: u64,
    mode: u32,
    linked: bool,
    metadata: NodeMetadata,
}

#[derive(Clone, Copy)]
struct Descriptor {
    node: usize,
    pipe_id: Option<usize>,
    offset: i64,
    readable: bool,
    writable: bool,
    append: bool,
    nonblocking: bool,
    close_on_exec: bool,
}

struct PipeBuffer {
    inode: u64,
    bytes: VecDeque<u8>,
    readers: usize,
    writers: usize,
}

#[derive(Clone)]
pub(crate) struct DirectoryEntry {
    pub(crate) inode: u64,
    pub(crate) mode: u32,
    pub(crate) name: Vec<u8>,
}

struct FileSystem {
    nodes: Vec<Option<Node>>,
    descriptors: [Option<Descriptor>; DESCRIPTOR_COUNT],
    standard_descriptors: [bool; 3],
    pipes: Vec<Option<PipeBuffer>>,
    root_metadata: Option<NodeMetadata>,
    next_inode: u64,
}

impl FileSystem {
    const fn new() -> Self {
        Self {
            nodes: Vec::new(),
            descriptors: [None; DESCRIPTOR_COUNT],
            standard_descriptors: [true; 3],
            pipes: Vec::new(),
            root_metadata: None,
            next_inode: 2,
        }
    }

    fn find_node(&self, path: &[u8]) -> Option<usize> {
        if path == b"/" {
            return Some(ROOT_NODE);
        }
        self.nodes
            .iter()
            .enumerate()
            .find_map(|(index, candidate)| {
                let node = candidate.as_ref()?;
                (node.linked && node.path.as_deref() == Some(path)).then_some(index)
            })
    }

    fn node_mode(&self, index: usize) -> Option<u32> {
        if index == ROOT_NODE {
            Some(
                S_IFDIR
                    | self
                        .root_metadata
                        .as_ref()
                        .map_or(0o755, |metadata| u32::from(metadata.mode())),
            )
        } else if index == TERMINAL_NODE {
            Some(S_IFCHR | 0o600)
        } else {
            self.nodes.get(index)?.as_ref().map(|node| node.mode)
        }
    }

    fn node_length(&self, index: usize) -> Option<usize> {
        if index == ROOT_NODE {
            Some(0)
        } else if index == TERMINAL_NODE {
            None
        } else {
            self.nodes.get(index)?.as_ref().map(|node| node.data.len())
        }
    }

    fn node_inode(&self, index: usize) -> Option<u64> {
        if index == ROOT_NODE {
            Some(1)
        } else {
            self.nodes.get(index)?.as_ref().map(|node| node.inode)
        }
    }

    fn read_directory(&self, file_descriptor: i32) -> Result<Vec<DirectoryEntry>, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if !descriptor.readable {
            return Err(EBADF);
        }
        if self.node_mode(descriptor.node).ok_or(EBADF)? & S_IFMT != S_IFDIR {
            return Err(ENOTDIR);
        }
        let directory_path = if descriptor.node == ROOT_NODE {
            b"/".as_slice()
        } else {
            self.nodes
                .get(descriptor.node)
                .and_then(Option::as_ref)
                .and_then(|node| node.path.as_deref())
                .ok_or(EBADF)?
        };
        let directory_inode = self.node_inode(descriptor.node).ok_or(EBADF)?;
        let parent_inode = if descriptor.node == ROOT_NODE {
            directory_inode
        } else {
            let parent = self.parent_node(directory_path)?;
            self.node_inode(parent).ok_or(ENOENT)?
        };
        let mut entries = Vec::new();
        entries
            .try_reserve(self.nodes.len().checked_add(2).ok_or(ENOMEM)?)
            .map_err(|_| ENOMEM)?;
        entries.push(DirectoryEntry {
            inode: directory_inode,
            mode: S_IFDIR,
            name: b".".to_vec(),
        });
        entries.push(DirectoryEntry {
            inode: parent_inode,
            mode: S_IFDIR,
            name: b"..".to_vec(),
        });
        for node in self.nodes.iter().flatten() {
            if !node.linked {
                continue;
            }
            let Some(path) = node.path.as_deref() else {
                continue;
            };
            let child = if directory_path == b"/" {
                path.strip_prefix(b"/")
            } else {
                path.strip_prefix(directory_path)
                    .and_then(|suffix| suffix.strip_prefix(b"/"))
            };
            let Some(name) = child.filter(|name| !name.is_empty() && !name.contains(&b'/')) else {
                continue;
            };
            if name.len() > 255 {
                return Err(ENAMETOOLONG);
            }
            let mut name_copy = Vec::new();
            name_copy.try_reserve_exact(name.len()).map_err(|_| ENOMEM)?;
            name_copy.extend_from_slice(name);
            entries.push(DirectoryEntry {
                inode: node.inode,
                mode: node.mode & S_IFMT,
                name: name_copy,
            });
        }
        Ok(entries)
    }

    fn directory_position(&self, file_descriptor: i32) -> Result<usize, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if self.node_mode(descriptor.node).ok_or(EBADF)? & S_IFMT != S_IFDIR {
            return Err(ENOTDIR);
        }
        usize::try_from(descriptor.offset).map_err(|_| EINVAL)
    }

    fn directory_path(&self, file_descriptor: i32) -> Result<Vec<u8>, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if self.node_mode(descriptor.node).ok_or(EBADF)? & S_IFMT != S_IFDIR {
            return Err(ENOTDIR);
        }
        if descriptor.node == ROOT_NODE {
            let mut path = Vec::new();
            path.try_reserve_exact(1).map_err(|_| ENOMEM)?;
            path.push(b'/');
            return Ok(path);
        }
        let path = self
            .nodes
            .get(descriptor.node)
            .and_then(Option::as_ref)
            .and_then(|node| node.path.as_deref())
            .ok_or(EBADF)?;
        let mut copy = Vec::new();
        copy.try_reserve_exact(path.len()).map_err(|_| ENOMEM)?;
        copy.extend_from_slice(path);
        Ok(copy)
    }

    fn set_directory_position(
        &mut self,
        file_descriptor: i32,
        position: usize,
    ) -> Result<(), i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let mut descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if self.node_mode(descriptor.node).ok_or(EBADF)? & S_IFMT != S_IFDIR {
            return Err(ENOTDIR);
        }
        descriptor.offset = i64::try_from(position).map_err(|_| EINVAL)?;
        self.descriptors[slot] = Some(descriptor);
        Ok(())
    }

    fn node_metadata(&mut self, index: usize) -> Result<NodeMetadata, i32> {
        if index == ROOT_NODE {
            if self.root_metadata.is_none() {
                self.root_metadata = Some(
                    NodeMetadata::with_owner_and_mode(UserId::ROOT, GroupId::ROOT, 0o755)
                        .map_err(metadata_errno)?,
                );
            }
            return self.root_metadata.clone().ok_or(ENOENT);
        }
        self.nodes
            .get(index)
            .and_then(Option::as_ref)
            .map(|node| node.metadata.clone())
            .ok_or(ENOENT)
    }

    fn metadata_for_fd(&self, file_descriptor: i32) -> Result<NodeMetadata, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if descriptor.node == ROOT_NODE {
            return Err(EBADF);
        }
        self.nodes
            .get(descriptor.node)
            .and_then(Option::as_ref)
            .map(|node| node.metadata.clone())
            .ok_or(EBADF)
    }

    fn descriptor_slot(&self, file_descriptor: i32) -> Result<usize, i32> {
        if file_descriptor < FIRST_DESCRIPTOR {
            return Err(EBADF);
        }
        let slot = (file_descriptor - FIRST_DESCRIPTOR) as usize;
        if slot >= DESCRIPTOR_COUNT || self.descriptors[slot].is_none() {
            return Err(EBADF);
        }
        Ok(slot)
    }

    fn parent_node(&self, path: &[u8]) -> Result<usize, i32> {
        let slash = path.iter().rposition(|byte| *byte == b'/').unwrap_or(0);
        let parent = if slash == 0 {
            b"/".as_slice()
        } else {
            &path[..slash]
        };
        let Some(index) = self.find_node(parent) else {
            return Err(ENOENT);
        };
        if self.node_mode(index).unwrap_or(0) & S_IFMT != S_IFDIR {
            return Err(ENOTDIR);
        }
        Ok(index)
    }

    fn check_search_permissions(
        &mut self,
        path: &[u8],
        credentials: &Credentials,
    ) -> Result<(), i32> {
        let root = self.node_metadata(ROOT_NODE)?;
        root.check_access(credentials, 0o1, true)
            .map_err(metadata_errno)?;
        for (index, byte) in path.iter().copied().enumerate().skip(1) {
            if byte != b'/' {
                continue;
            }
            let directory = self.find_node(&path[..index]).ok_or(ENOENT)?;
            if self.node_mode(directory).ok_or(ENOENT)? & S_IFMT != S_IFDIR {
                return Err(ENOTDIR);
            }
            self.node_metadata(directory)?
                .check_access(credentials, 0o1, true)
                .map_err(metadata_errno)?;
        }
        let parent = self.parent_node(path)?;
        self.node_metadata(parent)?
            .check_access(credentials, 0o1, true)
            .map_err(metadata_errno)
    }

    fn validate_path_components(
        &mut self,
        input: &[u8],
        base: &[u8],
        credentials: &Credentials,
    ) -> Result<(), i32> {
        if input == b"/dev/tty" || input == b"/dev/console" || input == b"/dev/net/tun" {
            return Ok(());
        }
        let mut current = if input.first() == Some(&b'/') {
            b"/".to_vec()
        } else {
            base.to_vec()
        };
        let mut components = input
            .split(|byte| *byte == b'/')
            .filter(|component| !component.is_empty())
            .peekable();
        self.check_directory_search(&current, credentials)?;
        while let Some(component) = components.next() {
            self.check_directory_search(&current, credentials)?;
            match component {
                b"." => {}
                b".." => pop_path_component(&mut current),
                component => {
                    let separator = usize::from(current.len() > 1);
                    if current.len() + separator + component.len() > PATH_MAX {
                        return Err(ENAMETOOLONG);
                    }
                    if separator != 0 {
                        current.push(b'/');
                    }
                    current.extend_from_slice(component);
                    if components.peek().is_some() {
                        let is_device_directory =
                            (current.as_slice() == b"/dev"
                                && components.peek().is_some_and(|next| {
                                    *next == b"tty" || *next == b"console" || *next == b"net"
                                }))
                                || (current.as_slice() == b"/dev/net"
                                    && components.peek().is_some_and(|next| *next == b"tun"));
                        if !is_device_directory {
                            let directory = self.find_node(&current).ok_or(ENOENT)?;
                            if self.node_mode(directory).ok_or(ENOENT)? & S_IFMT != S_IFDIR {
                                return Err(ENOTDIR);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn check_directory_search(
        &mut self,
        path: &[u8],
        credentials: &Credentials,
    ) -> Result<(), i32> {
        if path == b"/dev" || path == b"/dev/net" {
            return Ok(());
        }
        let directory = self.find_node(path).ok_or(ENOENT)?;
        if self.node_mode(directory).ok_or(ENOENT)? & S_IFMT != S_IFDIR {
            return Err(ENOTDIR);
        }
        self.node_metadata(directory)?
            .check_access(credentials, 0o1, true)
            .map_err(metadata_errno)
    }

    fn parent_allows_mutation(&mut self, path: &[u8]) -> Result<(), i32> {
        let parent = self.parent_node(path)?;
        if self.node_metadata(parent)?.flags() & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND) != 0 {
            return Err(EPERM);
        }
        Ok(())
    }

    fn allocate_node(
        &mut self,
        path: Option<Vec<u8>>,
        mode: u32,
        linked: bool,
        owner: UserId,
        group: GroupId,
    ) -> Result<usize, i32> {
        let metadata = NodeMetadata::with_owner_and_mode(owner, group, (mode & 0o777) as u16)
            .map_err(metadata_errno)?;
        let node = Node {
            path,
            data: Vec::new(),
            open_count: 0,
            inode: self.next_inode,
            mode,
            linked,
            metadata,
        };
        self.next_inode = self.next_inode.wrapping_add(1).max(2);
        if let Some(index) = self.nodes.iter().position(Option::is_none) {
            self.nodes[index] = Some(node);
            return Ok(index);
        }
        self.nodes.try_reserve(1).map_err(|_| ENOMEM)?;
        let index = self.nodes.len();
        self.nodes.push(Some(node));
        Ok(index)
    }

    fn open(
        &mut self,
        path: Vec<u8>,
        flags: i32,
        mode: u32,
        credentials: &Credentials,
    ) -> Result<i32, i32> {
        let access = flags & O_ACCMODE;
        if flags & !O_SUPPORTED != 0 || access == O_ACCMODE {
            return Err(EINVAL);
        }
        if flags & O_TRUNC != 0 && access == O_RDONLY {
            return Err(EINVAL);
        }
        self.check_search_permissions(&path, credentials)?;
        let slot = self
            .descriptors
            .iter()
            .position(Option::is_none)
            .ok_or(EMFILE)?;
        let existing = self.find_node(&path);
        if existing.is_some() && flags & O_CREAT != 0 && flags & O_EXCL != 0 {
            return Err(EEXIST);
        }
        let node_index = match existing {
            Some(index) => index,
            None => {
                if flags & O_CREAT == 0 || flags & O_DIRECTORY != 0 {
                    return Err(ENOENT);
                }
                if path == b"/" {
                    return Err(EISDIR);
                }
                let parent = self.parent_node(&path)?;
                self.parent_allows_mutation(&path)?;
                self.node_metadata(parent)?
                    .check_access(credentials, 0o3, true)
                    .map_err(metadata_errno)?;
                self.allocate_node(
                    Some(path),
                    S_IFREG | (mode & 0o777),
                    true,
                    credentials.effective_user(),
                    credentials.effective_group(),
                )?
            }
        };
        let node_mode = self.node_mode(node_index).ok_or(ENOENT)?;
        let node_type = node_mode & S_IFMT;
        if flags & O_DIRECTORY != 0 && node_type != S_IFDIR {
            return Err(ENOTDIR);
        }
        if node_type == S_IFDIR && (access != O_RDONLY || flags & O_TRUNC != 0) {
            return Err(EISDIR);
        }
        if existing.is_some() {
            let requested = match access {
                O_RDONLY => 0o4,
                O_WRONLY => 0o2,
                O_RDWR => 0o6,
                _ => return Err(EINVAL),
            };
            self.node_metadata(node_index)?
                .check_access(credentials, requested, node_type == S_IFDIR)
                .map_err(metadata_errno)?;
        }
        if flags & O_TRUNC != 0
            && self.node_metadata(node_index)?.flags()
                & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND)
                != 0
        {
            return Err(EPERM);
        }
        if node_index != ROOT_NODE {
            let node = self.nodes[node_index].as_mut().ok_or(ENOENT)?;
            if flags & O_TRUNC != 0 {
                node.data.clear();
            }
            node.open_count += 1;
        }
        let offset = if flags & O_APPEND != 0 {
            self.node_length(node_index).ok_or(ENOENT)? as i64
        } else {
            0
        };
        self.descriptors[slot] = Some(Descriptor {
            node: node_index,
            pipe_id: None,
            offset,
            readable: access != O_WRONLY,
            writable: access != O_RDONLY,
            append: flags & O_APPEND != 0,
            nonblocking: flags & O_NONBLOCK != 0,
            close_on_exec: flags & O_CLOEXEC != 0,
        });
        Ok(slot as i32 + FIRST_DESCRIPTOR)
    }

    fn open_terminal(&mut self, flags: i32) -> Result<i32, i32> {
        let access = flags & O_ACCMODE;
        if flags & !(O_ACCMODE | O_NONBLOCK | O_CLOEXEC | O_NOCTTY) != 0
            || access == O_ACCMODE
        {
            return Err(EINVAL);
        }
        let slot = self
            .descriptors
            .iter()
            .position(Option::is_none)
            .ok_or(EMFILE)?;
        self.descriptors[slot] = Some(Descriptor {
            node: TERMINAL_NODE,
            pipe_id: None,
            offset: 0,
            readable: access != O_WRONLY,
            writable: access != O_RDONLY,
            append: false,
            nonblocking: flags & O_NONBLOCK != 0,
            close_on_exec: flags & O_CLOEXEC != 0,
        });
        Ok(slot as i32 + FIRST_DESCRIPTOR)
    }

    fn create_pipe(&mut self, flags: i32) -> Result<(i32, i32), i32> {
        if flags & !(O_NONBLOCK | O_CLOEXEC) != 0 {
            return Err(EINVAL);
        }
        let mut free_slots = self
            .descriptors
            .iter()
            .enumerate()
            .filter_map(|(index, descriptor)| descriptor.is_none().then_some(index));
        let read_slot = free_slots.next().ok_or(EMFILE)?;
        let write_slot = free_slots.next().ok_or(EMFILE)?;
        let pipe_id = if let Some(index) = self.pipes.iter().position(Option::is_none) {
            index
        } else {
            self.pipes.try_reserve(1).map_err(|_| ENOMEM)?;
            self.pipes.push(None);
            self.pipes.len() - 1
        };
        let inode = self.next_inode;
        self.next_inode = self.next_inode.wrapping_add(1).max(2);
        self.pipes[pipe_id] = Some(PipeBuffer {
            inode,
            bytes: VecDeque::new(),
            readers: 1,
            writers: 1,
        });
        let nonblocking = flags & O_NONBLOCK != 0;
        let close_on_exec = flags & O_CLOEXEC != 0;
        self.descriptors[read_slot] = Some(Descriptor {
            node: PIPE_NODE,
            pipe_id: Some(pipe_id),
            offset: 0,
            readable: true,
            writable: false,
            append: false,
            nonblocking,
            close_on_exec,
        });
        self.descriptors[write_slot] = Some(Descriptor {
            node: PIPE_NODE,
            pipe_id: Some(pipe_id),
            offset: 0,
            readable: false,
            writable: true,
            append: false,
            nonblocking,
            close_on_exec,
        });
        Ok((read_slot as i32 + FIRST_DESCRIPTOR, write_slot as i32 + FIRST_DESCRIPTOR))
    }

    fn terminal_access(
        &self,
        file_descriptor: i32,
        require_read: bool,
        require_write: bool,
    ) -> Result<Option<bool>, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if descriptor.node != TERMINAL_NODE {
            return Ok(None);
        }
        if (require_read && !descriptor.readable) || (require_write && !descriptor.writable) {
            return Err(EBADF);
        }
        Ok(Some(descriptor.nonblocking))
    }

    fn is_terminal_descriptor(&self, file_descriptor: i32) -> Result<bool, i32> {
        if crate::kernel::tty::is_standard_terminal_descriptor(file_descriptor) {
            return if self.standard_descriptor_is_open(file_descriptor)? {
                Ok(true)
            } else {
                Err(EBADF)
            };
        }
        self.descriptor_slot(file_descriptor)
            .and_then(|slot| self.descriptors[slot].ok_or(EBADF))
            .map(|descriptor| descriptor.node == TERMINAL_NODE)
    }

    fn read(&mut self, file_descriptor: i32, buffer: *mut u8, length: usize) -> Result<usize, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let mut descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if !descriptor.readable {
            return Err(EBADF);
        }
        if let Some(pipe_id) = descriptor.pipe_id {
            let pipe = self
                .pipes
                .get_mut(pipe_id)
                .and_then(Option::as_mut)
                .ok_or(EBADF)?;
            if length == 0 {
                return Ok(0);
            }
            if pipe.bytes.is_empty() {
                return if pipe.writers == 0 {
                    Ok(0)
                } else {
                    Err(-(KernelError::Again as i32))
                };
            }
            let amount = length.min(pipe.bytes.len());
            for index in 0..amount {
                if let Some(byte) = pipe.bytes.pop_front() {
                    unsafe { write_volatile(buffer.add(index), byte) };
                }
            }
            return Ok(amount);
        }
        if self.node_mode(descriptor.node).ok_or(EBADF)? & S_IFMT == S_IFDIR {
            return Err(EISDIR);
        }
        if length == 0 {
            return Ok(0);
        }
        let node = self.nodes[descriptor.node].as_ref().ok_or(EBADF)?;
        if descriptor.offset < 0 || descriptor.offset as usize >= node.data.len() {
            return Ok(0);
        }
        let offset = descriptor.offset as usize;
        let amount = length.min(node.data.len() - offset);
        for index in 0..amount {
            unsafe { write_volatile(buffer.add(index), node.data[offset + index]) };
        }
        descriptor.offset += amount as i64;
        self.descriptors[slot] = Some(descriptor);
        Ok(amount)
    }

    fn pread(
        &self,
        file_descriptor: i32,
        buffer: *mut u8,
        length: usize,
        offset: i64,
    ) -> Result<usize, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if !descriptor.readable {
            return Err(EBADF);
        }
        if descriptor.node == TERMINAL_NODE || descriptor.pipe_id.is_some() {
            return Err(-(KernelError::Spipe as i32));
        }
        if self.node_mode(descriptor.node).ok_or(EBADF)? & S_IFMT == S_IFDIR {
            return Err(EISDIR);
        }
        if offset < 0 {
            return Err(EINVAL);
        }
        if length == 0 {
            return Ok(0);
        }
        let node = self.nodes.get(descriptor.node).and_then(Option::as_ref).ok_or(EBADF)?;
        let offset = usize::try_from(offset).map_err(|_| EINVAL)?;
        if offset >= node.data.len() {
            return Ok(0);
        }
        let amount = length.min(node.data.len() - offset);
        for index in 0..amount {
            unsafe { write_volatile(buffer.add(index), node.data[offset + index]) };
        }
        Ok(amount)
    }

    fn write(
        &mut self,
        file_descriptor: i32,
        buffer: *const u8,
        length: usize,
    ) -> Result<usize, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let mut descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if !descriptor.writable {
            return Err(EBADF);
        }
        if let Some(pipe_id) = descriptor.pipe_id {
            let pipe = self
                .pipes
                .get_mut(pipe_id)
                .and_then(Option::as_mut)
                .ok_or(EBADF)?;
            if length == 0 {
                return Ok(0);
            }
            if pipe.readers == 0 {
                return Err(-(KernelError::Pipe as i32));
            }
            let available = PIPE_CAPACITY.saturating_sub(pipe.bytes.len());
            if available == 0 || (length <= PIPE_BUF && available < length) {
                return Err(-(KernelError::Again as i32));
            }
            let amount = length.min(available);
            pipe.bytes.try_reserve(amount).map_err(|_| ENOMEM)?;
            for index in 0..amount {
                pipe.bytes.push_back(unsafe { read_volatile(buffer.add(index)) });
            }
            return Ok(amount);
        }
        if self.node_mode(descriptor.node).ok_or(EBADF)? & S_IFMT == S_IFDIR {
            return Err(EISDIR);
        }
        if length == 0 {
            return Ok(0);
        }
        if descriptor.node == ROOT_NODE {
            return Err(EISDIR);
        }
        let inode_flags = self.node_metadata(descriptor.node)?.flags();
        if inode_flags & INODE_FLAG_IMMUTABLE != 0 {
            return Err(EPERM);
        }
        let node = self.nodes[descriptor.node].as_mut().ok_or(EBADF)?;
        if descriptor.append || inode_flags & INODE_FLAG_APPEND != 0 {
            descriptor.offset = node.data.len() as i64;
        }
        if descriptor.offset < 0 || length > i64::MAX as usize {
            return Err(EFBIG);
        }
        let offset = descriptor.offset as usize;
        let end = offset.checked_add(length).ok_or(EFBIG)?;
        if end > i64::MAX as usize {
            return Err(EFBIG);
        }
        node.data
            .try_reserve(end.saturating_sub(node.data.len()))
            .map_err(|_| ENOMEM)?;
        if end > node.data.len() {
            node.data.resize(end, 0);
        }
        for index in 0..length {
            node.data[offset + index] = unsafe { read_volatile(buffer.add(index)) };
        }
        descriptor.offset = end as i64;
        self.descriptors[slot] = Some(descriptor);
        Ok(length)
    }

    fn pwrite(
        &mut self,
        file_descriptor: i32,
        buffer: *const u8,
        length: usize,
        offset: i64,
    ) -> Result<usize, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if !descriptor.writable {
            return Err(EBADF);
        }
        if descriptor.node == TERMINAL_NODE || descriptor.pipe_id.is_some() {
            return Err(-(KernelError::Spipe as i32));
        }
        if self.node_mode(descriptor.node).ok_or(EBADF)? & S_IFMT == S_IFDIR {
            return Err(EISDIR);
        }
        if offset < 0 || length > i64::MAX as usize {
            return Err(EINVAL);
        }
        if length == 0 {
            return Ok(0);
        }
        let start = usize::try_from(offset).map_err(|_| EFBIG)?;
        let end = start.checked_add(length).filter(|end| *end <= i64::MAX as usize).ok_or(EFBIG)?;
        let inode_flags = self.node_metadata(descriptor.node)?.flags();
        if inode_flags & INODE_FLAG_IMMUTABLE != 0 {
            return Err(EPERM);
        }
        if inode_flags & INODE_FLAG_APPEND != 0 {
            let node = self.nodes.get(descriptor.node).and_then(Option::as_ref).ok_or(EBADF)?;
            if start != node.data.len() {
                return Err(EPERM);
            }
        }
        let node = self.nodes.get_mut(descriptor.node).and_then(Option::as_mut).ok_or(EBADF)?;
        node.data
            .try_reserve(end.saturating_sub(node.data.len()))
            .map_err(|_| ENOMEM)?;
        if end > node.data.len() {
            node.data.resize(end, 0);
        }
        for index in 0..length {
            node.data[start + index] = unsafe { read_volatile(buffer.add(index)) };
        }
        Ok(length)
    }

    fn resize_file(&mut self, node_index: usize, length: usize) -> Result<(), i32> {
        if node_index == ROOT_NODE || node_index == TERMINAL_NODE {
            return Err(EINVAL);
        }
        if self.node_mode(node_index).ok_or(EBADF)? & S_IFMT != S_IFREG {
            return Err(EINVAL);
        }
        let flags = self.node_metadata(node_index)?.flags();
        if flags & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND) != 0 {
            return Err(EPERM);
        }
        if length > i64::MAX as usize {
            return Err(EFBIG);
        }
        let node = self.nodes.get_mut(node_index).and_then(Option::as_mut).ok_or(EBADF)?;
        if length > node.data.len() {
            node.data
                .try_reserve(length - node.data.len())
                .map_err(|_| ENOMEM)?;
            node.data.resize(length, 0);
        } else {
            node.data.truncate(length);
        }
        Ok(())
    }

    fn ftruncate(&mut self, file_descriptor: i32, length: i64) -> Result<(), i32> {
        if length < 0 {
            return Err(EINVAL);
        }
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if !descriptor.writable {
            return Err(EBADF);
        }
        self.resize_file(descriptor.node, length as usize)
    }

    fn truncate(
        &mut self,
        path: &[u8],
        length: i64,
        credentials: &Credentials,
    ) -> Result<(), i32> {
        if length < 0 {
            return Err(EINVAL);
        }
        self.check_search_permissions(path, credentials)?;
        let node = self.find_node(path).ok_or(ENOENT)?;
        self.node_metadata(node)?
            .check_access(credentials, 0o2, false)
            .map_err(metadata_errno)?;
        self.resize_file(node, length as usize)
    }

    fn posix_fallocate(
        &mut self,
        file_descriptor: i32,
        offset: i64,
        length: i64,
    ) -> Result<(), i32> {
        if offset < 0 || length < 0 {
            return Err(EINVAL);
        }
        let end = offset.checked_add(length).ok_or(EFBIG)?;
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if !descriptor.writable {
            return Err(EBADF);
        }
        if descriptor.pipe_id.is_some() || descriptor.node == TERMINAL_NODE {
            return Err(EINVAL);
        }
        let node = self.nodes.get(descriptor.node).and_then(Option::as_ref).ok_or(EBADF)?;
        if node.mode & S_IFMT != S_IFREG {
            return Err(EINVAL);
        }
        if node.data.len() < end as usize {
            self.resize_file(descriptor.node, end as usize)?;
        }
        Ok(())
    }

    fn seek(&mut self, file_descriptor: i32, offset: i64, origin: i32) -> Result<i64, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if descriptor.node == TERMINAL_NODE || descriptor.pipe_id.is_some() {
            return Err(-(KernelError::Spipe as i32));
        }
        let base = match origin {
            0 => 0,
            1 => descriptor.offset,
            2 => self.node_length(descriptor.node).ok_or(EBADF)? as i64,
            _ => return Err(EINVAL),
        };
        let position = base.checked_add(offset).ok_or(EINVAL)?;
        if position < 0 {
            return Err(EINVAL);
        }
        self.descriptors[slot] = Some(Descriptor {
            offset: position,
            ..descriptor
        });
        Ok(position)
    }

    fn close(&mut self, file_descriptor: i32) -> Result<(), i32> {
        if crate::kernel::tty::is_standard_terminal_descriptor(file_descriptor) {
            let descriptor = &mut self.standard_descriptors[file_descriptor as usize];
            if !*descriptor {
                return Err(EBADF);
            }
            *descriptor = false;
            return Ok(());
        }
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].take().ok_or(EBADF)?;
        if let Some(pipe_id) = descriptor.pipe_id {
            let empty = {
                let pipe = self
                    .pipes
                    .get_mut(pipe_id)
                    .and_then(Option::as_mut)
                    .ok_or(EBADF)?;
                if descriptor.readable {
                    pipe.readers = pipe.readers.saturating_sub(1);
                }
                if descriptor.writable {
                    pipe.writers = pipe.writers.saturating_sub(1);
                }
                pipe.readers == 0 && pipe.writers == 0
            };
            if empty {
                self.pipes[pipe_id] = None;
            }
            return Ok(());
        }
        if descriptor.node == TERMINAL_NODE {
            return Ok(());
        }
        if descriptor.node != ROOT_NODE {
            let node = self.nodes[descriptor.node].as_mut().ok_or(EBADF)?;
            node.open_count -= 1;
            if !node.linked && node.open_count == 0 {
                self.nodes[descriptor.node] = None;
            }
        }
        Ok(())
    }

    fn standard_descriptor_is_open(&self, file_descriptor: i32) -> Result<bool, i32> {
        if !crate::kernel::tty::is_standard_terminal_descriptor(file_descriptor) {
            return Err(EBADF);
        }
        self.standard_descriptors
            .get(file_descriptor as usize)
            .copied()
            .ok_or(EBADF)
    }

    fn descriptor_for_duplication(&self, file_descriptor: i32) -> Result<Descriptor, i32> {
        if crate::kernel::tty::is_standard_terminal_descriptor(file_descriptor) {
            if !self.standard_descriptor_is_open(file_descriptor)? {
                return Err(EBADF);
            }
            return Ok(Descriptor {
                node: TERMINAL_NODE,
                pipe_id: None,
                offset: 0,
                readable: file_descriptor == 0,
                writable: file_descriptor != 0,
                append: false,
                nonblocking: false,
                close_on_exec: false,
            });
        }
        let slot = self.descriptor_slot(file_descriptor)?;
        self.descriptors[slot].ok_or(EBADF)
    }

    fn duplicate(
        &mut self,
        file_descriptor: i32,
        minimum: i32,
        close_on_exec: bool,
    ) -> Result<i32, i32> {
        if minimum < 0 {
            return Err(EINVAL);
        }
        let source = self.descriptor_for_duplication(file_descriptor)?;
        let target_slot = self
            .descriptors
            .iter()
            .enumerate()
            .skip(minimum.saturating_sub(FIRST_DESCRIPTOR) as usize)
            .find_map(|(slot, descriptor)| descriptor.is_none().then_some(slot))
            .ok_or(EMFILE)?;
        self.add_descriptor_reference(source)?;
        self.descriptors[target_slot] = Some(Descriptor {
            close_on_exec,
            ..source
        });
        Ok(target_slot as i32 + FIRST_DESCRIPTOR)
    }

    fn duplicate_to(
        &mut self,
        file_descriptor: i32,
        new_descriptor: i32,
        close_on_exec: bool,
    ) -> Result<i32, i32> {
        let source = self.descriptor_for_duplication(file_descriptor)?;
        if file_descriptor == new_descriptor {
            if close_on_exec {
                return Err(EINVAL);
            }
            return Ok(new_descriptor);
        }
        if new_descriptor < FIRST_DESCRIPTOR {
            return Err(EBADF);
        }
        let target_slot = (new_descriptor - FIRST_DESCRIPTOR) as usize;
        if target_slot >= DESCRIPTOR_COUNT {
            return Err(EMFILE);
        }
        if self.descriptors[target_slot].is_some() {
            self.close(new_descriptor)?;
        }
        self.add_descriptor_reference(source)?;
        self.descriptors[target_slot] = Some(Descriptor {
            close_on_exec,
            ..source
        });
        Ok(new_descriptor)
    }

    fn add_descriptor_reference(&mut self, descriptor: Descriptor) -> Result<(), i32> {
        if let Some(pipe_id) = descriptor.pipe_id {
            let pipe = self
                .pipes
                .get_mut(pipe_id)
                .and_then(Option::as_mut)
                .ok_or(EBADF)?;
            let readers = if descriptor.readable {
                pipe.readers.checked_add(1).ok_or(EMFILE)?
            } else {
                pipe.readers
            };
            let writers = if descriptor.writable {
                pipe.writers.checked_add(1).ok_or(EMFILE)?
            } else {
                pipe.writers
            };
            pipe.readers = readers;
            pipe.writers = writers;
            return Ok(());
        }
        if matches!(descriptor.node, ROOT_NODE | TERMINAL_NODE) {
            return Ok(());
        }
        let node = self
            .nodes
            .get_mut(descriptor.node)
            .and_then(Option::as_mut)
            .ok_or(EBADF)?;
        node.open_count = node.open_count.checked_add(1).ok_or(EMFILE)?;
        Ok(())
    }

    fn descriptor_flags(&self, file_descriptor: i32) -> Result<i32, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        Ok(i32::from(descriptor.close_on_exec))
    }

    fn set_descriptor_flags(&mut self, file_descriptor: i32, flags: i32) -> Result<(), i32> {
        if flags & !1 != 0 {
            return Err(EINVAL);
        }
        let slot = self.descriptor_slot(file_descriptor)?;
        let mut descriptor = self.descriptors[slot].ok_or(EBADF)?;
        descriptor.close_on_exec = flags & 1 != 0;
        self.descriptors[slot] = Some(descriptor);
        Ok(())
    }

    fn status_flags(&self, file_descriptor: i32) -> Result<i32, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        let access = match (descriptor.readable, descriptor.writable) {
            (true, false) => O_RDONLY,
            (false, true) => O_WRONLY,
            (true, true) => O_RDWR,
            (false, false) => return Err(EBADF),
        };
        let append = if descriptor.append { O_APPEND } else { 0 };
        let nonblocking = if descriptor.nonblocking { O_NONBLOCK } else { 0 };
        Ok(access | append | nonblocking)
    }

    fn set_status_flags(&mut self, file_descriptor: i32, flags: i32) -> Result<(), i32> {
        if flags & !(O_ACCMODE | O_APPEND | O_NONBLOCK) != 0 {
            return Err(EINVAL);
        }
        let slot = self.descriptor_slot(file_descriptor)?;
        let mut descriptor = self.descriptors[slot].ok_or(EBADF)?;
        descriptor.append = flags & O_APPEND != 0;
        descriptor.nonblocking = flags & O_NONBLOCK != 0;
        self.descriptors[slot] = Some(descriptor);
        Ok(())
    }

    fn unlink(&mut self, path: &[u8], credentials: &Credentials) -> Result<(), i32> {
        self.check_search_permissions(path, credentials)?;
        let index = self.find_node(path).ok_or(ENOENT)?;
        let mode = self.node_mode(index).ok_or(ENOENT)?;
        if mode & S_IFMT == S_IFDIR {
            return Err(EISDIR);
        }
        self.parent_allows_mutation(path)?;
        let parent = self.parent_node(path)?;
        self.node_metadata(parent)?
            .check_access(credentials, 0o3, true)
            .map_err(metadata_errno)?;
        if self.node_metadata(index)?.flags() & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND) != 0 {
            return Err(EPERM);
        }
        let node = self.nodes[index].as_mut().ok_or(ENOENT)?;
        node.linked = false;
        if node.open_count == 0 {
            self.nodes[index] = None;
        }
        Ok(())
    }

    fn mkdir(
        &mut self,
        path: Vec<u8>,
        mode: u32,
        credentials: &Credentials,
    ) -> Result<(), i32> {
        self.check_search_permissions(&path, credentials)?;
        if self.find_node(&path).is_some() {
            return Err(EEXIST);
        }
        self.parent_allows_mutation(&path)?;
        let parent = self.parent_node(&path)?;
        self.node_metadata(parent)?
            .check_access(credentials, 0o3, true)
            .map_err(metadata_errno)?;
        self.allocate_node(
            Some(path),
            S_IFDIR | (mode & 0o777),
            true,
            credentials.effective_user(),
            credentials.effective_group(),
        )?;
        Ok(())
    }

    fn chmod(
        &mut self,
        path: &[u8],
        mode: u32,
        credentials: &Credentials,
    ) -> Result<(), i32> {
        if mode & !0o777 != 0 {
            return Err(EINVAL);
        }
        self.check_search_permissions(path, credentials)?;
        let index = self.find_node(path).ok_or(ENOENT)?;
        let metadata = self.node_metadata(index)?;
        if metadata.flags() & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND) != 0 {
            return Err(EPERM);
        }
        metadata
            .set_mode(credentials, mode as u16)
            .map_err(metadata_errno)?;
        if index != ROOT_NODE {
            let node = self.nodes[index].as_mut().ok_or(ENOENT)?;
            node.mode = (node.mode & S_IFMT) | mode;
        }
        Ok(())
    }

    fn fchmod(
        &mut self,
        file_descriptor: i32,
        mode: u32,
        credentials: &Credentials,
    ) -> Result<(), i32> {
        if mode & !0o777 != 0 {
            return Err(EINVAL);
        }
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if descriptor.node == TERMINAL_NODE {
            return Err(EBADF);
        }
        let metadata = self.node_metadata(descriptor.node)?;
        if metadata.flags() & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND) != 0 {
            return Err(EPERM);
        }
        metadata
            .set_mode(credentials, mode as u16)
            .map_err(metadata_errno)?;
        if descriptor.node != ROOT_NODE {
            let node = self.nodes[descriptor.node].as_mut().ok_or(EBADF)?;
            node.mode = (node.mode & S_IFMT) | mode;
        }
        Ok(())
    }

    fn chown(
        &mut self,
        path: &[u8],
        owner: u32,
        group: u32,
        credentials: &Credentials,
    ) -> Result<(), i32> {
        self.check_search_permissions(path, credentials)?;
        let index = self.find_node(path).ok_or(ENOENT)?;
        self.set_node_owner(index, owner, group, credentials)
    }

    fn fchown(
        &mut self,
        file_descriptor: i32,
        owner: u32,
        group: u32,
        credentials: &Credentials,
    ) -> Result<(), i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if descriptor.node == TERMINAL_NODE {
            return Err(EBADF);
        }
        self.set_node_owner(descriptor.node, owner, group, credentials)
    }

    fn set_node_owner(
        &mut self,
        index: usize,
        owner: u32,
        group: u32,
        credentials: &Credentials,
    ) -> Result<(), i32> {
        let metadata = self.node_metadata(index)?;
        if metadata.flags() & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND) != 0 {
            return Err(EPERM);
        }
        let owner = if owner == u32::MAX {
            metadata.owner()
        } else {
            UserId::new(owner)
        };
        let group = if group == u32::MAX {
            metadata.group()
        } else {
            GroupId::new(group)
        };
        if owner == metadata.owner() && group == metadata.group() {
            return Ok(());
        }
        metadata
            .set_owner_and_group(credentials, owner, group)
            .map_err(metadata_errno)
    }

    fn rmdir(&mut self, path: &[u8], credentials: &Credentials) -> Result<(), i32> {
        self.check_search_permissions(path, credentials)?;
        if path == b"/" {
            return Err(EBUSY);
        }
        let index = self.find_node(path).ok_or(ENOENT)?;
        if self.node_mode(index).ok_or(ENOENT)? & S_IFMT != S_IFDIR {
            return Err(ENOTDIR);
        }
        self.parent_allows_mutation(path)?;
        let parent = self.parent_node(path)?;
        self.node_metadata(parent)?
            .check_access(credentials, 0o3, true)
            .map_err(metadata_errno)?;
        if self.node_metadata(index)?.flags() & (INODE_FLAG_IMMUTABLE | INODE_FLAG_APPEND) != 0 {
            return Err(EPERM);
        }
        for candidate in self.nodes.iter().flatten() {
            if !candidate.linked {
                continue;
            }
            if let Some(child_path) = candidate.path.as_deref() {
                if child_path.starts_with(path) && child_path.get(path.len()) == Some(&b'/') {
                    return Err(ENOTEMPTY);
                }
            }
        }
        let node = self.nodes[index].as_mut().ok_or(ENOENT)?;
        node.linked = false;
        if node.open_count == 0 {
            self.nodes[index] = None;
        }
        Ok(())
    }

    fn fd_access(
        &self,
        file_descriptor: i32,
        require_read: bool,
        require_write: bool,
    ) -> Result<(), i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if (require_read && !descriptor.readable) || (require_write && !descriptor.writable) {
            return Err(EBADF);
        }
        Ok(())
    }

    fn stat_node(&self, index: usize) -> Result<CStat, i32> {
        if index == TERMINAL_NODE {
            return Ok(CStat {
                device: 1,
                inode: 4,
                links: 1,
                mode: S_IFCHR | 0o600,
                user: 0,
                group: 0,
                pad: 0,
                special_device: 0,
                size: 0,
                block_size: 4096,
                blocks: 0,
                accessed: Timespec { seconds: 0, nanoseconds: 0 },
                modified: Timespec { seconds: 0, nanoseconds: 0 },
                changed: Timespec { seconds: 0, nanoseconds: 0 },
                reserved: [0; 3],
            });
        }
        let (inode, mode, size, links, user, group) = if index == ROOT_NODE {
            (
                1,
                S_IFDIR
                    | self
                        .root_metadata
                        .as_ref()
                        .map_or(0o755, |metadata| u32::from(metadata.mode())),
                0,
                1,
                self.root_metadata.as_ref().map_or(0, |metadata| metadata.owner().raw()),
                self.root_metadata.as_ref().map_or(0, |metadata| metadata.group().raw()),
            )
        } else {
            let node = self
                .nodes
                .get(index)
                .and_then(Option::as_ref)
                .ok_or(ENOENT)?;
            (
                node.inode,
                node.mode,
                node.data.len(),
                u64::from(node.linked),
                node.metadata.owner().raw(),
                node.metadata.group().raw(),
            )
        };
        Ok(CStat {
            device: 1,
            inode,
            links,
            mode,
            user,
            group,
            pad: 0,
            special_device: 0,
            size: size as i64,
            block_size: 4096,
            blocks: (size / 512 + if size % 512 != 0 { 1 } else { 0 }) as i64,
            accessed: Timespec {
                seconds: 0,
                nanoseconds: 0,
            },
            modified: Timespec {
                seconds: 0,
                nanoseconds: 0,
            },
            changed: Timespec {
                seconds: 0,
                nanoseconds: 0,
            },
            reserved: [0; 3],
        })
    }

    fn stat_descriptor(&self, file_descriptor: i32) -> Result<CStat, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if let Some(pipe_id) = descriptor.pipe_id {
            let pipe = self
                .pipes
                .get(pipe_id)
                .and_then(Option::as_ref)
                .ok_or(EBADF)?;
            return Ok(CStat {
                device: 1,
                inode: pipe.inode,
                links: 1,
                mode: S_IFIFO | 0o600,
                user: 0,
                group: 0,
                pad: 0,
                special_device: 0,
                size: pipe.bytes.len() as i64,
                block_size: 4096,
                blocks: 0,
                accessed: Timespec { seconds: 0, nanoseconds: 0 },
                modified: Timespec { seconds: 0, nanoseconds: 0 },
                changed: Timespec { seconds: 0, nanoseconds: 0 },
                reserved: [0; 3],
            });
        }
        self.stat_node(descriptor.node)
    }
}

static FILESYSTEM: IrqSpinLock<FileSystem> = IrqSpinLock::new(FileSystem::new());

pub(crate) fn read_directory(file_descriptor: i32) -> Result<Vec<DirectoryEntry>, i32> {
    FILESYSTEM.lock().read_directory(file_descriptor)
}

pub(crate) fn directory_position(file_descriptor: i32) -> Result<usize, i32> {
    FILESYSTEM.lock().directory_position(file_descriptor)
}

pub(crate) fn set_directory_position(file_descriptor: i32, position: usize) -> Result<(), i32> {
    FILESYSTEM
        .lock()
        .set_directory_position(file_descriptor, position)
}

pub(crate) fn duplicate(
    file_descriptor: i32,
    minimum: i32,
    close_on_exec: bool,
) -> Result<i32, i32> {
    FILESYSTEM
        .lock()
        .duplicate(file_descriptor, minimum, close_on_exec)
}

pub(crate) fn duplicate_to(
    file_descriptor: i32,
    new_descriptor: i32,
    close_on_exec: bool,
) -> Result<i32, i32> {
    FILESYSTEM
        .lock()
        .duplicate_to(file_descriptor, new_descriptor, close_on_exec)
}

pub(crate) fn descriptor_flags(file_descriptor: i32) -> Result<i32, i32> {
    FILESYSTEM.lock().descriptor_flags(file_descriptor)
}

pub(crate) fn set_descriptor_flags(file_descriptor: i32, flags: i32) -> Result<(), i32> {
    FILESYSTEM
        .lock()
        .set_descriptor_flags(file_descriptor, flags)
}

pub(crate) fn status_flags(file_descriptor: i32) -> Result<i32, i32> {
    FILESYSTEM.lock().status_flags(file_descriptor)
}

pub(crate) fn set_status_flags(file_descriptor: i32, flags: i32) -> Result<(), i32> {
    FILESYSTEM.lock().set_status_flags(file_descriptor, flags)
}

pub(crate) fn create_pipe(flags: i32) -> Result<(i32, i32), i32> {
    FILESYSTEM.lock().create_pipe(flags)
}

pub(crate) fn pipe_nonblocking(file_descriptor: i32) -> Result<Option<bool>, i32> {
    let filesystem = FILESYSTEM.lock();
    let slot = filesystem.descriptor_slot(file_descriptor)?;
    let descriptor = filesystem.descriptors[slot].ok_or(EBADF)?;
    Ok(descriptor.pipe_id.map(|_| descriptor.nonblocking))
}

pub(crate) fn is_pipe_descriptor(file_descriptor: i32) -> bool {
    pipe_nonblocking(file_descriptor).is_ok_and(|nonblocking| nonblocking.is_some())
}

pub(crate) fn file_identity(path: *const c_char) -> Result<(u64, u64), i32> {
    let path = unsafe { normalize_path(path) }?;
    let credentials = current_credentials()?;
    let mut filesystem = FILESYSTEM.lock();
    filesystem.check_search_permissions(&path, &credentials)?;
    let node = filesystem.find_node(&path).ok_or(ENOENT)?;
    let status = filesystem.stat_node(node)?;
    Ok((status.device, status.inode))
}

pub(crate) fn positioned_read(
    file_descriptor: i32,
    buffer: *mut u8,
    length: usize,
    offset: i64,
) -> Result<usize, i32> {
    FILESYSTEM
        .lock()
        .pread(file_descriptor, buffer, length, offset)
}

pub(crate) fn positioned_write(
    file_descriptor: i32,
    buffer: *const u8,
    length: usize,
    offset: i64,
) -> Result<usize, i32> {
    FILESYSTEM
        .lock()
        .pwrite(file_descriptor, buffer, length, offset)
}

pub(crate) fn truncate_descriptor(file_descriptor: i32, length: i64) -> Result<(), i32> {
    FILESYSTEM.lock().ftruncate(file_descriptor, length)
}

pub(crate) fn truncate_path(path: *const c_char, length: i64) -> Result<(), i32> {
    if length < 0 {
        return Err(EINVAL);
    }
    let path = unsafe { normalize_path(path) }?;
    let credentials = current_credentials()?;
    FILESYSTEM
        .lock()
        .truncate(&path, length, &credentials)
}

pub(crate) fn posix_fallocate(
    file_descriptor: i32,
    offset: i64,
    length: i64,
) -> Result<(), i32> {
    FILESYSTEM
        .lock()
        .posix_fallocate(file_descriptor, offset, length)
}

fn current_directory_path() -> Result<Vec<u8>, i32> {
    crate::kernel::scheduler::thread::ThreadManager::current_process()
        .and_then(|process| process.current_directory())
        .map_err(|error| -(error as i32))
}

fn normalize_path_from(
    input: &[u8],
    base: &[u8],
    credentials: &Credentials,
) -> Result<Vec<u8>, i32> {
    if input.is_empty() {
        return Err(ENOENT);
    }
    if input.len() > PATH_MAX || base.first() != Some(&b'/') {
        return Err(ENAMETOOLONG);
    }
    if input != b"/dev/tty" && input != b"/dev/console" && input != b"/dev/net/tun" {
        FILESYSTEM
            .lock()
            .validate_path_components(input, base, credentials)?;
    }
    let reserve = base
        .len()
        .checked_add(input.len())
        .and_then(|length| length.checked_add(1))
        .ok_or(ENAMETOOLONG)?;
    let mut output = Vec::new();
    output.try_reserve(reserve).map_err(|_| ENOMEM)?;
    if input.first() == Some(&b'/') {
        output.push(b'/');
    } else {
        output.extend_from_slice(base);
    }
    for segment in input.split(|byte| *byte == b'/') {
        if segment.is_empty() || segment == b"." {
            continue;
        }
        if segment == b".." {
            if output.len() > 1 {
                while output.len() > 1 && output.last() != Some(&b'/') {
                    output.pop();
                }
                if output.len() > 1 {
                    output.pop();
                }
            }
            continue;
        }
        let separator = usize::from(output.len() > 1);
        if output.len() + separator + segment.len() > PATH_MAX {
            return Err(ENAMETOOLONG);
        }
        if separator != 0 {
            output.push(b'/');
        }
        output.extend_from_slice(segment);
    }
    Ok(output)
}

fn pop_path_component(path: &mut Vec<u8>) {
    if path.len() > 1 {
        while path.len() > 1 && path.last() != Some(&b'/') {
            path.pop();
        }
        if path.len() > 1 {
            path.pop();
        }
    }
}

unsafe fn normalize_path_at(
    directory_descriptor: i32,
    path: *const c_char,
) -> Result<Vec<u8>, i32> {
    if path.is_null() {
        return Err(EFAULT);
    }
    let input = unsafe { core::ffi::CStr::from_ptr(path) }.to_bytes();
    let credentials = current_credentials()?;
    let base = if directory_descriptor == AT_FDCWD {
        if input.first() == Some(&b'/') {
            b"/".to_vec()
        } else {
            current_directory_path()?
        }
    } else if input.first() == Some(&b'/') {
        b"/".to_vec()
    } else {
        FILESYSTEM.lock().directory_path(directory_descriptor)?
    };
    normalize_path_from(input, &base, &credentials)
}

pub(crate) fn poll_readiness(file_descriptor: i32, events: i16) -> i16 {
    const POLLIN: i16 = 0x001;
    const POLLOUT: i16 = 0x004;
    const POLLNVAL: i16 = 0x020;

    if network::is_socket_fd(file_descriptor) {
        return match network::readiness(file_descriptor, events) {
            Ok(readiness) => readiness,
            Err(KernelError::BadFd) => POLLNVAL,
            Err(_) => 0x008,
        };
    }
    if network::is_packet_fd(file_descriptor) {
        return match network::packet_readiness(file_descriptor, events) {
            Ok(readiness) => readiness,
            Err(KernelError::BadFd) => POLLNVAL,
            Err(_) => 0x008,
        };
    }
    if file_descriptor == 0 {
        if !standard_descriptor_is_open(file_descriptor).unwrap_or(false) {
            return POLLNVAL;
        }
        return if events & POLLIN != 0 && crate::kernel::tty::input_ready() {
            POLLIN
        } else {
            0
        };
    }
    if file_descriptor == 1 || file_descriptor == 2 {
        if !standard_descriptor_is_open(file_descriptor).unwrap_or(false) {
            return POLLNVAL;
        }
        return if events & POLLOUT != 0 && crate::kernel::tty::output_ready() {
            POLLOUT
        } else {
            0
        };
    }
    let descriptor = {
        let filesystem = FILESYSTEM.lock();
        let Ok(slot) = filesystem.descriptor_slot(file_descriptor) else {
            return POLLNVAL;
        };
        let Some(descriptor) = filesystem.descriptors[slot] else {
            return POLLNVAL;
        };
        descriptor
    };
    let mut readiness = 0;
    if descriptor.node == TERMINAL_NODE {
        if events & POLLIN != 0 && descriptor.readable && crate::kernel::tty::input_ready() {
            readiness |= POLLIN;
        }
        if events & POLLOUT != 0 && descriptor.writable && crate::kernel::tty::output_ready() {
            readiness |= POLLOUT;
        }
        return readiness;
    }
    if let Some(pipe_id) = descriptor.pipe_id {
        const POLLERR: i16 = 0x008;
        const POLLHUP: i16 = 0x010;
        let filesystem = FILESYSTEM.lock();
        let Some(pipe) = filesystem.pipes.get(pipe_id).and_then(Option::as_ref) else {
            return POLLNVAL;
        };
        if descriptor.readable && (!pipe.bytes.is_empty() || pipe.writers == 0) {
            readiness |= POLLIN;
        }
        if descriptor.writable && pipe.readers != 0 && pipe.bytes.len() < PIPE_CAPACITY {
            readiness |= POLLOUT;
        }
        if descriptor.readable && pipe.writers == 0 {
            readiness |= POLLHUP;
        }
        if descriptor.writable && pipe.readers == 0 {
            readiness |= POLLERR;
        }
        return readiness;
    }
    if events & POLLIN != 0 && descriptor.readable {
        readiness |= POLLIN;
    }
    if events & POLLOUT != 0 && descriptor.writable {
        readiness |= POLLOUT;
    }
    readiness
}

fn set_errno(error: i32) {
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
}

fn fail(error: i32) -> i32 {
    set_errno(error);
    -1
}

unsafe fn normalize_path(path: *const c_char) -> Result<Vec<u8>, i32> {
    if path.is_null() {
        return Err(EFAULT);
    }
    let input = unsafe { core::ffi::CStr::from_ptr(path) }.to_bytes();
    let credentials = current_credentials()?;
    let current_directory = if input.first() == Some(&b'/') {
        None
    } else {
        Some(current_directory_path()?)
    };
    normalize_path_from(
        input,
        current_directory.as_deref().unwrap_or(b"/"),
        &credentials,
    )
}

fn with_path(path: *const c_char, operation: impl FnOnce(Vec<u8>) -> Result<i32, i32>) -> i32 {
    let normalized = match unsafe { normalize_path(path) } {
        Ok(path) => path,
        Err(error) => return fail(error),
    };
    match operation(normalized) {
        Ok(value) => value,
        Err(error) => fail(error),
    }
}

fn with_path_at(
    directory_descriptor: i32,
    path: *const c_char,
    operation: impl FnOnce(Vec<u8>) -> Result<i32, i32>,
) -> i32 {
    let normalized = match unsafe { normalize_path_at(directory_descriptor, path) } {
        Ok(path) => path,
        Err(error) => return fail(error),
    };
    match operation(normalized) {
        Ok(value) => value,
        Err(error) => fail(error),
    }
}

fn path_c_string(path: &[u8]) -> Result<Vec<u8>, i32> {
    let length = path.len().checked_add(1).ok_or(ENAMETOOLONG)?;
    let mut output = Vec::new();
    output.try_reserve_exact(length).map_err(|_| ENOMEM)?;
    output.extend_from_slice(path);
    output.push(0);
    Ok(output)
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_open(path: *const c_char, flags: i32, mode: u32) -> i32 {
    with_path(path, |path| {
        if path.as_slice() == b"/dev/tty" || path.as_slice() == b"/dev/console" {
            let access = flags & O_ACCMODE;
            if flags & !(O_ACCMODE | O_NONBLOCK | O_CLOEXEC | O_NOCTTY) != 0
                || access == O_ACCMODE
            {
                return Err(EINVAL);
            }
            let process = crate::kernel::scheduler::thread::ThreadManager::current_process()
                .map_err(|error| -(error as i32))?;
            if path.as_slice() == b"/dev/tty" {
                process
                    .controlling_terminal_session()
                    .map_err(|_| -(KernelError::Nxio as i32))?;
            }
            if path.as_slice() == b"/dev/console" {
                let credentials = process.credentials().map_err(metadata_errno)?;
                let requested = match access {
                    O_RDONLY => 0o4,
                    O_WRONLY => 0o2,
                    O_RDWR => 0o6,
                    _ => return Err(EINVAL),
                };
                NodeMetadata::with_owner_and_mode(UserId::ROOT, GroupId::ROOT, 0o600)
                    .map_err(metadata_errno)?
                    .check_access(&credentials, requested, false)
                    .map_err(metadata_errno)?;
            }
            let descriptor = FILESYSTEM.lock().open_terminal(flags)?;
            if path.as_slice() == b"/dev/console" {
                process.acquire_console_terminal(flags & O_NOCTTY != 0);
            }
            return Ok(descriptor);
        }
        if path.as_slice() == b"/dev/net/tun" {
            if flags & O_ACCMODE != O_RDWR
                || flags & !(O_ACCMODE | O_NONBLOCK | O_CLOEXEC | O_NOCTTY) != 0
            {
                return Err(EINVAL);
            }
            return network::open_tun_device(flags & O_NONBLOCK != 0)
                .map_err(|error| -(error as i32));
        }
        let credentials = current_credentials()?;
        let process = crate::kernel::scheduler::thread::ThreadManager::current_process()
            .map_err(|error| -(error as i32))?;
        FILESYSTEM
            .lock()
            .open(path, flags, mode & !process.umask(), &credentials)
    })
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_openat(
    directory_descriptor: i32,
    path: *const c_char,
    flags: i32,
    mode: u32,
) -> i32 {
    with_path_at(directory_descriptor, path, |path| {
        let path = path_c_string(&path)?;
        let descriptor = unsafe { kz_rust_open(path.as_ptr().cast(), flags, mode) };
        if descriptor < 0 {
            Err(unsafe { crate::kernel::scheduler::thread::__errno_location().read() })
        } else {
            Ok(descriptor)
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_statat(
    directory_descriptor: i32,
    path: *const c_char,
    output: *mut c_void,
) -> i32 {
    with_path_at(directory_descriptor, path, |path| {
        let path = path_c_string(&path)?;
        if unsafe { stat(path.as_ptr().cast(), output) } == 0 {
            Ok(0)
        } else {
            Err(unsafe { crate::kernel::scheduler::thread::__errno_location().read() })
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_mkdirat(
    directory_descriptor: i32,
    path: *const c_char,
    mode: u32,
) -> i32 {
    with_path_at(directory_descriptor, path, |path| {
        let path = path_c_string(&path)?;
        let result = unsafe { mkdir(path.as_ptr().cast(), mode) };
        if result == 0 {
            Ok(0)
        } else {
            Err(unsafe { crate::kernel::scheduler::thread::__errno_location().read() })
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_unlinkat(
    directory_descriptor: i32,
    path: *const c_char,
    flags: i32,
) -> i32 {
    with_path_at(directory_descriptor, path, |path| {
        let path = path_c_string(&path)?;
        let result = match flags {
            0 => unsafe { unlink(path.as_ptr().cast()) },
            0x200 => unsafe { rmdir(path.as_ptr().cast()) },
            _ => return Err(EINVAL),
        };
        if result == 0 {
            Ok(0)
        } else {
            Err(unsafe { crate::kernel::scheduler::thread::__errno_location().read() })
        }
    })
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_chmod(path: *const c_char, mode: u32) -> i32 {
    with_path(path, |path| {
        let credentials = current_credentials()?;
        FILESYSTEM.lock().chmod(&path, mode, &credentials).map(|()| 0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_chmodat(
    directory_descriptor: i32,
    path: *const c_char,
    mode: u32,
) -> i32 {
    with_path_at(directory_descriptor, path, |path| {
        let credentials = current_credentials()?;
        FILESYSTEM.lock().chmod(&path, mode, &credentials).map(|()| 0)
    })
}

#[no_mangle]
pub extern "C" fn kz_rust_fchmod(file_descriptor: i32, mode: u32) -> i32 {
    let credentials = match current_credentials() {
        Ok(credentials) => credentials,
        Err(error) => return fail(error),
    };
    match FILESYSTEM
        .lock()
        .fchmod(file_descriptor, mode, &credentials)
    {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}
#[no_mangle]
pub unsafe extern "C" fn kz_rust_chown(
    path: *const c_char,
    owner: u32,
    group: u32,
) -> i32 {
    with_path(path, |path| {
        let credentials = current_credentials()?;
        FILESYSTEM
            .lock()
            .chown(&path, owner, group, &credentials)
            .map(|()| 0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_chownat(
    directory_descriptor: i32,
    path: *const c_char,
    owner: u32,
    group: u32,
) -> i32 {
    with_path_at(directory_descriptor, path, |path| {
        let credentials = current_credentials()?;
        FILESYSTEM
            .lock()
            .chown(&path, owner, group, &credentials)
            .map(|()| 0)
    })
}

#[no_mangle]
pub extern "C" fn kz_rust_fchown(file_descriptor: i32, owner: u32, group: u32) -> i32 {
    let credentials = match current_credentials() {
        Ok(credentials) => credentials,
        Err(error) => return fail(error),
    };
    match FILESYSTEM
        .lock()
        .fchown(file_descriptor, owner, group, &credentials)
    {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

fn change_current_directory(path: Vec<u8>) -> Result<(), i32> {
    let process = crate::kernel::scheduler::thread::ThreadManager::current_process()
        .map_err(|error| -(error as i32))?;
    let credentials = process.credentials().map_err(metadata_errno)?;
    {
        let mut filesystem = FILESYSTEM.lock();
        filesystem.check_search_permissions(&path, &credentials)?;
        let index = filesystem.find_node(&path).ok_or(ENOENT)?;
        if filesystem.node_mode(index).ok_or(ENOENT)? & S_IFMT != S_IFDIR {
            return Err(ENOTDIR);
        }
        filesystem
            .node_metadata(index)?
            .check_access(&credentials, 0o1, true)
            .map_err(metadata_errno)?;
    }
    process
        .set_current_directory(&path)
        .map_err(|error| -(error as i32))
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_chdir(path: *const c_char) -> i32 {
    with_path(path, |path| change_current_directory(path).map(|()| 0))
}

#[no_mangle]
pub extern "C" fn kz_rust_fchdir(file_descriptor: i32) -> i32 {
    let path = match FILESYSTEM.lock().directory_path(file_descriptor) {
        Ok(path) => path,
        Err(error) => return fail(error),
    };
    match change_current_directory(path) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

pub(crate) fn is_terminal_descriptor(file_descriptor: i32) -> Result<bool, i32> {
    FILESYSTEM.lock().is_terminal_descriptor(file_descriptor)
}

fn standard_descriptor_is_open(file_descriptor: i32) -> Result<bool, i32> {
    FILESYSTEM.lock().standard_descriptor_is_open(file_descriptor)
}

fn current_credentials() -> Result<Credentials, i32> {
    ThreadManager::current_process()
        .and_then(|process| process.credentials())
        .map_err(metadata_errno)
}

fn restart_interrupted_io() -> bool {
    crate::kernel::runtime::runtime_posix::signal::dispatch_pending_signals().unwrap_or(true)
}

fn dispatch_signals_before_io_return() {
    let _ = crate::kernel::runtime::runtime_posix::signal::dispatch_pending_signals();
}

fn set_interrupted_io_errno() {
    set_errno(-(KernelError::Intr as i32));
}

fn wake_pipe_waiters() {
    if let Ok(process) = ThreadManager::current_process() {
        ThreadManager::wake_blocked_process_threads(process.pid());
    }
}

fn wait_for_blocking_pipe(file_descriptor: i32) -> bool {
    matches!(pipe_nonblocking(file_descriptor), Ok(Some(false)))
}

#[no_mangle]
pub unsafe extern "C" fn read(file_descriptor: i32, buffer: *mut c_void, length: usize) -> isize {
    if crate::kernel::tty::is_standard_terminal_descriptor(file_descriptor)
        && (file_descriptor != 0 || !standard_descriptor_is_open(file_descriptor).unwrap_or(false))
    {
        set_errno(EBADF);
        return -1;
    }
    if buffer.is_null() && length != 0 {
        set_errno(EFAULT);
        return -1;
    }
    if length > isize::MAX as usize {
        set_errno(EINVAL);
        return -1;
    }
    if length == 0 {
        return 0;
    }
    if file_descriptor == 0 {
        let output = unsafe { core::slice::from_raw_parts_mut(buffer.cast::<u8>(), length) };
        loop {
            match crate::kernel::tty::read_console(output, false) {
                Ok(amount) => {
                    dispatch_signals_before_io_return();
                    return amount as isize;
                }
                Err(KernelError::Intr) => {
                    if restart_interrupted_io() {
                        continue;
                    }
                    set_interrupted_io_errno();
                    return -1;
                }
                Err(error) => {
                    set_errno(-(error as i32));
                    return -1;
                }
            }
        }
    }
    if file_descriptor < FIRST_DESCRIPTOR && file_descriptor != 0 {
        set_errno(EBADF);
        return -1;
    }
    if network::is_socket_fd(file_descriptor) {
        let output = unsafe { core::slice::from_raw_parts_mut(buffer.cast::<u8>(), length) };
        return match network::receive(file_descriptor, output) {
            Ok(amount) => {
                dispatch_signals_before_io_return();
                amount as isize
            }
            Err(error) => {
                set_errno(-(error as i32));
                -1
            }
        };
    }
    if network::is_packet_fd(file_descriptor) {
        let output = unsafe { core::slice::from_raw_parts_mut(buffer.cast::<u8>(), length) };
        loop {
            network::pump_packet_network();
            match network::read_packet_fd(file_descriptor, output) {
                Ok(amount) => {
                    dispatch_signals_before_io_return();
                    return amount as isize;
                }
                Err(KernelError::Again) => match network::packet_nonblocking(file_descriptor) {
                    Ok(true) => {
                        set_errno(-(KernelError::Again as i32));
                        return -1;
                    }
                    Ok(false) => {
                        if ThreadManager::current_signal_wake_pending() {
                            if restart_interrupted_io() {
                                continue;
                            }
                            set_interrupted_io_errno();
                            return -1;
                        }
                        crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
                    }
                    Err(error) => {
                        set_errno(-(error as i32));
                        return -1;
                    }
                },
                Err(error) => {
                    set_errno(-(error as i32));
                    return -1;
                }
            }
        }
    }
    let terminal_read = { FILESYSTEM.lock().terminal_access(file_descriptor, true, false) };
    match terminal_read {
        Ok(Some(nonblocking)) => {
            let output = unsafe { core::slice::from_raw_parts_mut(buffer.cast::<u8>(), length) };
            loop {
                match crate::kernel::tty::read_console(output, nonblocking) {
                    Ok(amount) => {
                        dispatch_signals_before_io_return();
                        return amount as isize;
                    }
                    Err(KernelError::Intr) => {
                        if restart_interrupted_io() {
                            continue;
                        }
                        set_interrupted_io_errno();
                        return -1;
                    }
                    Err(error) => {
                        set_errno(-(error as i32));
                        return -1;
                    }
                }
            }
        }
        Ok(None) => {}
        Err(error) => return fail(error) as isize,
    }
    if file_descriptor < FIRST_DESCRIPTOR {
        set_errno(EBADF);
        return -1;
    }
    loop {
        let result = {
            FILESYSTEM
                .lock()
                .read(file_descriptor, buffer.cast::<u8>(), length)
        };
        match result {
            Ok(amount) => {
                if is_pipe_descriptor(file_descriptor) {
                    wake_pipe_waiters();
                }
                dispatch_signals_before_io_return();
                return amount as isize;
            }
            Err(EAGAIN) if wait_for_blocking_pipe(file_descriptor) => {
                if ThreadManager::current_signal_wake_pending() && !restart_interrupted_io() {
                    set_interrupted_io_errno();
                    return -1;
                }
                if !ThreadManager::switch_current(crate::kernel::scheduler::thread::ThreadState::Blocked) {
                    crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
                }
            }
            Err(error) => {
                set_errno(error);
                return -1;
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn write(
    file_descriptor: i32,
    buffer: *const c_void,
    length: usize,
) -> isize {
    if crate::kernel::tty::is_standard_terminal_descriptor(file_descriptor)
        && (file_descriptor == 0 || !standard_descriptor_is_open(file_descriptor).unwrap_or(false))
    {
        set_errno(EBADF);
        return -1;
    }
    if buffer.is_null() && length != 0 {
        set_errno(EFAULT);
        return -1;
    }
    if length > isize::MAX as usize {
        set_errno(EINVAL);
        return -1;
    }
    if length == 0 {
        return 0;
    }
    if file_descriptor == 1 || file_descriptor == 2 {
        return write_console(buffer.cast::<u8>(), length, false);
    }
    if file_descriptor < FIRST_DESCRIPTOR {
        set_errno(EBADF);
        return -1;
    }
    if network::is_socket_fd(file_descriptor) {
        let input = unsafe { core::slice::from_raw_parts(buffer.cast::<u8>(), length) };
        return match network::send(file_descriptor, input) {
            Ok(amount) => amount as isize,
            Err(error) => {
                set_errno(-(error as i32));
                -1
            }
        };
    }
    if network::is_packet_fd(file_descriptor) {
        let input = unsafe { core::slice::from_raw_parts(buffer.cast::<u8>(), length) };
        loop {
            match network::write_packet_fd(file_descriptor, input) {
                Ok(amount) => return amount as isize,
                Err(KernelError::Again) => match network::packet_nonblocking(file_descriptor) {
                    Ok(true) => {
                        set_errno(-(KernelError::Again as i32));
                        return -1;
                    }
                    Ok(false) => {
                        if ThreadManager::current_signal_wake_pending() {
                            if restart_interrupted_io() {
                                continue;
                            }
                            set_interrupted_io_errno();
                            return -1;
                        }
                        crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
                    }
                    Err(error) => {
                        set_errno(-(error as i32));
                        return -1;
                    }
                },
                Err(error) => {
                    set_errno(-(error as i32));
                    return -1;
                }
            }
        }
    }
    let terminal_write = { FILESYSTEM.lock().terminal_access(file_descriptor, false, true) };
    match terminal_write {
        Ok(Some(nonblocking)) => return write_console(buffer.cast::<u8>(), length, nonblocking),
        Ok(None) => {}
        Err(error) => return fail(error) as isize,
    }
    if file_descriptor < FIRST_DESCRIPTOR {
        set_errno(EBADF);
        return -1;
    }
    loop {
        let result = {
            FILESYSTEM
                .lock()
                .write(file_descriptor, buffer.cast::<u8>(), length)
        };
        match result {
            Ok(amount) => {
                if is_pipe_descriptor(file_descriptor) {
                    wake_pipe_waiters();
                }
                return amount as isize;
            }
            Err(EAGAIN) if wait_for_blocking_pipe(file_descriptor) => {
                if ThreadManager::current_signal_wake_pending() && !restart_interrupted_io() {
                    set_interrupted_io_errno();
                    return -1;
                }
                if !ThreadManager::switch_current(crate::kernel::scheduler::thread::ThreadState::Blocked) {
                    crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
                }
            }
            Err(error) => {
                set_errno(error);
                return -1;
            }
        }
    }
}

fn write_console(buffer: *const u8, length: usize, nonblocking: bool) -> isize {
    let input = unsafe { core::slice::from_raw_parts(buffer, length) };
    loop {
        match crate::kernel::tty::write_console(input, nonblocking) {
            Ok(written) => {
                dispatch_signals_before_io_return();
                return written as isize;
            }
            Err(KernelError::Intr) => {
                if restart_interrupted_io() {
                    continue;
                }
                set_interrupted_io_errno();
                return -1;
            }
            Err(error) => {
                set_errno(-(error as i32));
                return -1;
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn close(file_descriptor: i32) -> i32 {
    if (0..=2).contains(&file_descriptor) {
        return match FILESYSTEM.lock().close(file_descriptor) {
            Ok(()) => 0,
            Err(error) => fail(error),
        };
    }
    if network::is_socket_fd(file_descriptor) {
        return match network::close(file_descriptor) {
            Ok(()) => 0,
            Err(error) => {
                set_errno(-(error as i32));
                -1
            }
        };
    }
    if network::is_packet_fd(file_descriptor) {
        return match network::close_packet_fd(file_descriptor) {
            Ok(()) => 0,
            Err(error) => {
                set_errno(-(error as i32));
                -1
            }
        };
    }
    let pipe = is_pipe_descriptor(file_descriptor);
    match FILESYSTEM.lock().close(file_descriptor) {
        Ok(()) => {
            if pipe {
                wake_pipe_waiters();
            }
            0
        }
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub extern "C" fn lseek(file_descriptor: i32, offset: i64, origin: i32) -> i64 {
    if network::is_socket_fd(file_descriptor) || network::is_packet_fd(file_descriptor) {
        if network::has_socket_fd(file_descriptor) || network::has_packet_fd(file_descriptor) {
            set_errno(-(KernelError::Spipe as i32));
        } else {
            set_errno(EBADF);
        }
        return -1;
    }
    match FILESYSTEM.lock().seek(file_descriptor, offset, origin) {
        Ok(position) => position,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn unlink(path: *const c_char) -> i32 {
    let normalized = match unsafe { normalize_path(path) } {
        Ok(path) => path,
        Err(error) => return fail(error),
    };
    let credentials = match current_credentials() {
        Ok(credentials) => credentials,
        Err(error) => return fail(error),
    };
    match FILESYSTEM.lock().unlink(&normalized, &credentials) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn mkdir(path: *const c_char, mode: u32) -> i32 {
    with_path(path, |path| {
        let credentials = current_credentials()?;
        let process = crate::kernel::scheduler::thread::ThreadManager::current_process()
            .map_err(|error| -(error as i32))?;
        FILESYSTEM
            .lock()
            .mkdir(path, mode & !process.umask(), &credentials)
            .map(|()| 0)
    })
}

#[no_mangle]
pub unsafe extern "C" fn rmdir(path: *const c_char) -> i32 {
    let normalized = match unsafe { normalize_path(path) } {
        Ok(path) => path,
        Err(error) => return fail(error),
    };
    let credentials = match current_credentials() {
        Ok(credentials) => credentials,
        Err(error) => return fail(error),
    };
    match FILESYSTEM.lock().rmdir(&normalized, &credentials) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

fn fill_status(output: *mut c_void, status: CStat) {
    unsafe { output.cast::<CStat>().write(status) };
}

#[no_mangle]
pub unsafe extern "C" fn fstat(file_descriptor: i32, output: *mut c_void) -> i32 {
    if output.is_null() {
        return fail(EFAULT);
    }
    let status = if network::has_socket_fd(file_descriptor) {
        Ok(CStat {
            device: 1,
            inode: file_descriptor as u64,
            links: 1,
            mode: S_IFSOCK | 0o777,
            user: 0,
            group: 0,
            pad: 0,
            special_device: 0,
            size: 0,
            block_size: 4096,
            blocks: 0,
            accessed: Timespec { seconds: 0, nanoseconds: 0 },
            modified: Timespec { seconds: 0, nanoseconds: 0 },
            changed: Timespec { seconds: 0, nanoseconds: 0 },
            reserved: [0; 3],
        })
    } else if network::has_packet_fd(file_descriptor) {
        Ok(CStat {
            device: 1,
            inode: file_descriptor as u64,
            links: 1,
            mode: S_IFCHR | 0o600,
            user: 0,
            group: 0,
            pad: 0,
            special_device: 0,
            size: 0,
            block_size: 4096,
            blocks: 0,
            accessed: Timespec { seconds: 0, nanoseconds: 0 },
            modified: Timespec { seconds: 0, nanoseconds: 0 },
            changed: Timespec { seconds: 0, nanoseconds: 0 },
            reserved: [0; 3],
        })
    } else if (0..=2).contains(&file_descriptor) {
        if !standard_descriptor_is_open(file_descriptor).unwrap_or(false) {
            return fail(EBADF);
        }
        Ok(CStat {
            device: 1,
            inode: file_descriptor as u64 + 1,
            links: 1,
            mode: S_IFCHR | 0o666,
            user: 0,
            group: 0,
            pad: 0,
            special_device: 0,
            size: 0,
            block_size: 4096,
            blocks: 0,
            accessed: Timespec {
                seconds: 0,
                nanoseconds: 0,
            },
            modified: Timespec {
                seconds: 0,
                nanoseconds: 0,
            },
            changed: Timespec {
                seconds: 0,
                nanoseconds: 0,
            },
            reserved: [0; 3],
        })
    } else {
        let filesystem = FILESYSTEM.lock();
        let slot = filesystem.descriptor_slot(file_descriptor);
        slot.and_then(|_| filesystem.stat_descriptor(file_descriptor))
    };
    match status {
        Ok(status) => {
            fill_status(output, status);
            0
        }
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn fstat64(file_descriptor: i32, output: *mut c_void) -> i32 {
    unsafe { fstat(file_descriptor, output) }
}

#[no_mangle]
pub unsafe extern "C" fn stat(path: *const c_char, output: *mut c_void) -> i32 {
    if output.is_null() {
        return fail(EFAULT);
    }
    let normalized = match unsafe { normalize_path(path) } {
        Ok(path) => path,
        Err(error) => return fail(error),
    };
    if normalized == b"/dev/tty" {
        let process = match ThreadManager::current_process() {
            Ok(process) => process,
            Err(error) => return fail(metadata_errno(error)),
        };
        if let Err(error) = process.controlling_terminal_session() {
            return fail(metadata_errno(error));
        }
        let status = FILESYSTEM.lock().stat_node(TERMINAL_NODE);
        return match status {
            Ok(status) => {
                fill_status(output, status);
                0
            }
            Err(error) => fail(error),
        };
    }
    if normalized == b"/dev/console" {
        let status = FILESYSTEM.lock().stat_node(TERMINAL_NODE);
        return match status {
            Ok(status) => {
                fill_status(output, status);
                0
            }
            Err(error) => fail(error),
        };
    }
    let credentials = match current_credentials() {
        Ok(credentials) => credentials,
        Err(error) => return fail(error),
    };
    let status = {
        let mut filesystem = FILESYSTEM.lock();
        filesystem
            .check_search_permissions(&normalized, &credentials)
            .and_then(|()| {
                filesystem
                    .find_node(&normalized)
                    .ok_or(ENOENT)
                    .and_then(|index| filesystem.stat_node(index))
            })
    };
    match status {
        Ok(status) => {
            fill_status(output, status);
            0
        }
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn stat64(path: *const c_char, output: *mut c_void) -> i32 {
    unsafe { stat(path, output) }
}

pub(super) fn tmpfile_descriptor() -> Result<i32, i32> {
    let credentials = current_credentials()?;
    let mut filesystem = FILESYSTEM.lock();
    let slot = filesystem
        .descriptors
        .iter()
        .position(Option::is_none)
        .ok_or(EMFILE)?;
    let node = filesystem.allocate_node(
        None,
        S_IFREG | 0o600,
        false,
        credentials.effective_user(),
        credentials.effective_group(),
    )?;
    filesystem.nodes[node].as_mut().ok_or(ENOMEM)?.open_count = 1;
    filesystem.descriptors[slot] = Some(Descriptor {
        node,
        pipe_id: None,
        offset: 0,
        readable: true,
        writable: true,
        append: false,
        nonblocking: false,
        close_on_exec: false,
    });
    Ok(slot as i32 + FIRST_DESCRIPTOR)
}

pub(super) fn fd_access(
    file_descriptor: i32,
    require_read: bool,
    require_write: bool,
) -> Result<(), i32> {
    FILESYSTEM
        .lock()
        .fd_access(file_descriptor, require_read, require_write)
}

pub(super) fn status_for_fd(file_descriptor: i32) -> Result<CStat, i32> {
    if (0..=2).contains(&file_descriptor) {
        return Ok(CStat {
            device: 1,
            inode: file_descriptor as u64 + 1,
            links: 1,
            mode: S_IFCHR | 0o666,
            user: 0,
            group: 0,
            pad: 0,
            special_device: 0,
            size: 0,
            block_size: 4096,
            blocks: 0,
            accessed: Timespec {
                seconds: 0,
                nanoseconds: 0,
            },
            modified: Timespec {
                seconds: 0,
                nanoseconds: 0,
            },
            changed: Timespec {
                seconds: 0,
                nanoseconds: 0,
            },
            reserved: [0; 3],
        });
    }
    let filesystem = FILESYSTEM.lock();
    let slot = filesystem.descriptor_slot(file_descriptor)?;
    let descriptor = filesystem.descriptors[slot].ok_or(EBADF)?;
    filesystem.stat_node(descriptor.node)
}

pub(super) fn set_errno_for_stdio(error: i32) {
    set_errno(error);
}

fn metadata_errno(error: KernelError) -> i32 {
    if error == KernelError::Perm {
        EPERM
    } else {
        -(error as i32)
    }
}

fn metadata_for_path(path: *const c_char) -> Result<NodeMetadata, i32> {
    let path = unsafe { normalize_path(path) }?;
    let credentials = current_credentials()?;
    let mut filesystem = FILESYSTEM.lock();
    filesystem.check_search_permissions(&path, &credentials)?;
    let node = filesystem.find_node(&path).ok_or(ENOENT)?;
    filesystem.node_metadata(node)
}

fn metadata_for_fd(file_descriptor: i32) -> Result<NodeMetadata, i32> {
    FILESYSTEM.lock().metadata_for_fd(file_descriptor)
}

unsafe fn xattr_name(name: *const c_char) -> Result<Vec<u8>, i32> {
    if name.is_null() {
        return Err(EFAULT);
    }
    let name = unsafe { CStr::from_ptr(name) }.to_bytes();
    if name.is_empty() || name.len() > XATTR_NAME_MAX {
        return Err(EINVAL);
    }
    let mut copy = Vec::new();
    copy.try_reserve_exact(name.len()).map_err(|_| ENOMEM)?;
    copy.extend_from_slice(name);
    Ok(copy)
}

unsafe fn xattr_value(value: *const c_void, size: usize) -> Result<Vec<u8>, i32> {
    if size > XATTR_SIZE_MAX {
        return Err(E2BIG);
    }
    if value.is_null() && size != 0 {
        return Err(EFAULT);
    }
    let mut copy = Vec::new();
    copy.try_reserve_exact(size).map_err(|_| ENOMEM)?;
    for index in 0..size {
        copy.push(unsafe { read_volatile(value.cast::<u8>().add(index)) });
    }
    Ok(copy)
}

fn copy_xattr_value(value: &[u8], output: *mut c_void, size: usize) -> Result<usize, i32> {
    if size == 0 {
        return Ok(value.len());
    }
    if output.is_null() {
        return Err(EFAULT);
    }
    if size < value.len() {
        return Err(ERANGE);
    }
    for (index, byte) in value.iter().copied().enumerate() {
        unsafe { write_volatile(output.cast::<u8>().add(index), byte) };
    }
    Ok(value.len())
}

fn copy_xattr_names(names: &[Vec<u8>], output: *mut c_char, size: usize) -> Result<usize, i32> {
    let length = names.iter().try_fold(0usize, |length, name| {
        length
            .checked_add(name.len())
            .and_then(|length| length.checked_add(1))
                .ok_or(E2BIG)
    })?;
            if length > XATTR_SIZE_MAX {
            return Err(E2BIG);
            }
    if size == 0 {
        return Ok(length);
    }
    if output.is_null() {
        return Err(EFAULT);
    }
    if size < length {
        return Err(ERANGE);
    }
    let mut offset = 0;
    for name in names {
        for byte in name {
            unsafe { write_volatile(output.cast::<u8>().add(offset), *byte) };
            offset += 1;
        }
        unsafe { write_volatile(output.cast::<u8>().add(offset), 0) };
        offset += 1;
    }
    Ok(length)
}

fn xattr_length_result(result: Result<usize, i32>) -> isize {
    match result {
        Ok(length) => isize::try_from(length).unwrap_or_else(|_| {
            set_errno(EFBIG);
            -1
        }),
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

fn xattr_status_result(result: Result<(), i32>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

fn get_xattr_path(
    path: *const c_char,
    name: *const c_char,
    output: *mut c_void,
    size: usize,
) -> Result<usize, i32> {
    let metadata = metadata_for_path(path)?;
    let name = unsafe { xattr_name(name) }?;
    let value = metadata.get_xattr(&name).map_err(metadata_errno)?;
    copy_xattr_value(&value, output, size)
}

fn get_xattr_fd(
    file_descriptor: i32,
    name: *const c_char,
    output: *mut c_void,
    size: usize,
) -> Result<usize, i32> {
    let metadata = metadata_for_fd(file_descriptor)?;
    let name = unsafe { xattr_name(name) }?;
    let value = metadata.get_xattr(&name).map_err(metadata_errno)?;
    copy_xattr_value(&value, output, size)
}

fn list_xattrs_path(path: *const c_char, output: *mut c_char, size: usize) -> Result<usize, i32> {
    let metadata = metadata_for_path(path)?;
    let names = metadata.list_xattrs().map_err(metadata_errno)?;
    copy_xattr_names(&names, output, size)
}

fn list_xattrs_fd(
    file_descriptor: i32,
    output: *mut c_char,
    size: usize,
) -> Result<usize, i32> {
    let metadata = metadata_for_fd(file_descriptor)?;
    let names = metadata.list_xattrs().map_err(metadata_errno)?;
    copy_xattr_names(&names, output, size)
}

fn set_xattr_path(
    path: *const c_char,
    name: *const c_char,
    value: *const c_void,
    size: usize,
    flags: i32,
) -> Result<(), i32> {
    let metadata = metadata_for_path(path)?;
    let credentials = current_credentials()?;
    let name = unsafe { xattr_name(name) }?;
    let value = unsafe { xattr_value(value, size) }?;
    metadata
        .set_xattr(&credentials, &name, &value, flags)
        .map_err(metadata_errno)
}

fn set_xattr_fd(
    file_descriptor: i32,
    name: *const c_char,
    value: *const c_void,
    size: usize,
    flags: i32,
) -> Result<(), i32> {
    let metadata = metadata_for_fd(file_descriptor)?;
    let credentials = current_credentials()?;
    let name = unsafe { xattr_name(name) }?;
    let value = unsafe { xattr_value(value, size) }?;
    metadata
        .set_xattr(&credentials, &name, &value, flags)
        .map_err(metadata_errno)
}

fn remove_xattr_path(path: *const c_char, name: *const c_char) -> Result<(), i32> {
    let metadata = metadata_for_path(path)?;
    let credentials = current_credentials()?;
    let name = unsafe { xattr_name(name) }?;
    metadata
        .remove_xattr(&credentials, &name)
        .map_err(metadata_errno)
}

fn remove_xattr_fd(file_descriptor: i32, name: *const c_char) -> Result<(), i32> {
    let metadata = metadata_for_fd(file_descriptor)?;
    let credentials = current_credentials()?;
    let name = unsafe { xattr_name(name) }?;
    metadata
        .remove_xattr(&credentials, &name)
        .map_err(metadata_errno)
}

#[no_mangle]
pub unsafe extern "C" fn getxattr(
    path: *const c_char,
    name: *const c_char,
    value: *mut c_void,
    size: usize,
) -> isize {
    xattr_length_result(get_xattr_path(path, name, value, size))
}

#[no_mangle]
pub unsafe extern "C" fn lgetxattr(
    path: *const c_char,
    name: *const c_char,
    value: *mut c_void,
    size: usize,
) -> isize {
    unsafe { getxattr(path, name, value, size) }
}

#[no_mangle]
pub unsafe extern "C" fn fgetxattr(
    file_descriptor: i32,
    name: *const c_char,
    value: *mut c_void,
    size: usize,
) -> isize {
    xattr_length_result(get_xattr_fd(file_descriptor, name, value, size))
}

#[no_mangle]
pub unsafe extern "C" fn listxattr(
    path: *const c_char,
    list: *mut c_char,
    size: usize,
) -> isize {
    xattr_length_result(list_xattrs_path(path, list, size))
}

#[no_mangle]
pub unsafe extern "C" fn llistxattr(
    path: *const c_char,
    list: *mut c_char,
    size: usize,
) -> isize {
    unsafe { listxattr(path, list, size) }
}

#[no_mangle]
pub unsafe extern "C" fn flistxattr(
    file_descriptor: i32,
    list: *mut c_char,
    size: usize,
) -> isize {
    xattr_length_result(list_xattrs_fd(file_descriptor, list, size))
}

#[no_mangle]
pub unsafe extern "C" fn setxattr(
    path: *const c_char,
    name: *const c_char,
    value: *const c_void,
    size: usize,
    flags: i32,
) -> i32 {
    xattr_status_result(set_xattr_path(path, name, value, size, flags))
}

#[no_mangle]
pub unsafe extern "C" fn lsetxattr(
    path: *const c_char,
    name: *const c_char,
    value: *const c_void,
    size: usize,
    flags: i32,
) -> i32 {
    unsafe { setxattr(path, name, value, size, flags) }
}

#[no_mangle]
pub unsafe extern "C" fn fsetxattr(
    file_descriptor: i32,
    name: *const c_char,
    value: *const c_void,
    size: usize,
    flags: i32,
) -> i32 {
    xattr_status_result(set_xattr_fd(file_descriptor, name, value, size, flags))
}

#[no_mangle]
pub unsafe extern "C" fn removexattr(path: *const c_char, name: *const c_char) -> i32 {
    xattr_status_result(remove_xattr_path(path, name))
}

#[no_mangle]
pub unsafe extern "C" fn lremovexattr(path: *const c_char, name: *const c_char) -> i32 {
    unsafe { removexattr(path, name) }
}

#[no_mangle]
pub unsafe extern "C" fn fremovexattr(file_descriptor: i32, name: *const c_char) -> i32 {
    xattr_status_result(remove_xattr_fd(file_descriptor, name))
}

const fn file_ioctl(direction: usize, number: usize) -> usize {
    (direction << 30)
        | (core::mem::size_of::<usize>() << 16)
        | ((b'f' as usize) << 8)
        | number
}

pub(crate) const FS_IOC_GETFLAGS: usize = file_ioctl(2, 1);
pub(crate) const FS_IOC_SETFLAGS: usize = file_ioctl(1, 2);

#[no_mangle]
pub unsafe extern "C" fn kz_rust_ioctl(
    file_descriptor: i32,
    request: usize,
    argument: *mut c_void,
) -> i32 {
    if network::ioctl_requires_admin(request) {
        match current_credentials() {
            Ok(credentials) if credentials.is_root() => {}
            Ok(_) => return fail(EPERM),
            Err(error) => return fail(error),
        }
    }
    if network::is_packet_fd(file_descriptor) {
        let result = match network::tun_ioctl(file_descriptor, request, argument) {
            Err(KernelError::NoSys) => network::interface_ioctl(request, argument),
            result => result,
        };
        return match result {
            Ok(()) => 0,
            Err(error) => fail(-(error as i32)),
        };
    }
    if network::is_socket_fd(file_descriptor) {
        return match network::interface_ioctl(request, argument) {
            Ok(()) => 0,
            Err(error) => fail(-(error as i32)),
        };
    }
    if request != FS_IOC_GETFLAGS && request != FS_IOC_SETFLAGS {
        return fail(ENOTTY);
    }
    if argument.is_null() {
        return fail(EFAULT);
    }
    let metadata = match metadata_for_fd(file_descriptor) {
        Ok(metadata) => metadata,
        Err(error) => return fail(error),
    };
    if request == FS_IOC_GETFLAGS {
        unsafe { write_volatile(argument.cast::<usize>(), metadata.flags() as usize) };
        return 0;
    }
    let flags = unsafe { read_volatile(argument.cast::<usize>()) };
    let Ok(flags) = u32::try_from(flags) else {
        return fail(EINVAL);
    };
    let credentials = match current_credentials() {
        Ok(credentials) => credentials,
        Err(error) => return fail(error),
    };
    match metadata.set_flags(&credentials, flags).map_err(metadata_errno) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

