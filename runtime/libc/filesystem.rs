/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::ffi::{c_char, c_void, CStr};
use core::ptr::{read_volatile, write_volatile};

use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::error::Error as KernelError;
use crate::kernel::vfs::{NodeMetadata, INODE_FLAG_APPEND, INODE_FLAG_IMMUTABLE};

use super::errno::{
    E2BIG, EBADF, EBUSY, EEXIST, EFAULT, EFBIG, EINVAL, EISDIR, EMFILE, ENAMETOOLONG, ENOENT,
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
const S_IFDIR: u32 = 0o040000;
const S_IFCHR: u32 = 0o020000;
const S_IFREG: u32 = 0o100000;
const S_IFSOCK: u32 = 0o140000;
const PATH_MAX: usize = 4096;
const DESCRIPTOR_COUNT: usize = 64;
const FIRST_DESCRIPTOR: i32 = 3;
const ROOT_NODE: usize = usize::MAX;
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
    offset: i64,
    readable: bool,
    writable: bool,
    append: bool,
}

struct FileSystem {
    nodes: Vec<Option<Node>>,
    descriptors: [Option<Descriptor>; DESCRIPTOR_COUNT],
    root_metadata: Option<NodeMetadata>,
    next_inode: u64,
}

impl FileSystem {
    const fn new() -> Self {
        Self {
            nodes: Vec::new(),
            descriptors: [None; DESCRIPTOR_COUNT],
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
            Some(S_IFDIR | 0o755)
        } else {
            self.nodes.get(index)?.as_ref().map(|node| node.mode)
        }
    }

    fn node_length(&self, index: usize) -> Option<usize> {
        if index == ROOT_NODE {
            Some(0)
        } else {
            self.nodes.get(index)?.as_ref().map(|node| node.data.len())
        }
    }

    fn node_metadata(&mut self, index: usize) -> Result<NodeMetadata, i32> {
        if index == ROOT_NODE {
            return Ok(self
                .root_metadata
                .get_or_insert_with(NodeMetadata::new)
                .clone());
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

    fn parent_is_directory(&self, path: &[u8]) -> Result<(), i32> {
        self.parent_node(path).map(|_| ())
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
    ) -> Result<usize, i32> {
        let node = Node {
            path,
            data: Vec::new(),
            open_count: 0,
            inode: self.next_inode,
            mode,
            linked,
            metadata: NodeMetadata::new(),
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

    fn open(&mut self, path: Vec<u8>, flags: i32, mode: u32) -> Result<i32, i32> {
        let access = flags & O_ACCMODE;
        if flags & !O_SUPPORTED != 0 || access == O_ACCMODE {
            return Err(EINVAL);
        }
        if flags & O_TRUNC != 0 && access == O_RDONLY {
            return Err(EINVAL);
        }
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
                self.parent_is_directory(&path)?;
                self.allocate_node(Some(path), S_IFREG | (mode & 0o777), true)?
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
            offset,
            readable: access != O_WRONLY,
            writable: access != O_RDONLY,
            append: flags & O_APPEND != 0,
        });
        Ok(slot as i32 + FIRST_DESCRIPTOR)
    }

    fn read(&mut self, file_descriptor: i32, buffer: *mut u8, length: usize) -> Result<usize, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let mut descriptor = self.descriptors[slot].ok_or(EBADF)?;
        if !descriptor.readable {
            return Err(EBADF);
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

    fn seek(&mut self, file_descriptor: i32, offset: i64, origin: i32) -> Result<i64, i32> {
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].ok_or(EBADF)?;
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
        let slot = self.descriptor_slot(file_descriptor)?;
        let descriptor = self.descriptors[slot].take().ok_or(EBADF)?;
        if descriptor.node != ROOT_NODE {
            let node = self.nodes[descriptor.node].as_mut().ok_or(EBADF)?;
            node.open_count -= 1;
            if !node.linked && node.open_count == 0 {
                self.nodes[descriptor.node] = None;
            }
        }
        Ok(())
    }

    fn unlink(&mut self, path: &[u8]) -> Result<(), i32> {
        let index = self.find_node(path).ok_or(ENOENT)?;
        let mode = self.node_mode(index).ok_or(ENOENT)?;
        if mode & S_IFMT == S_IFDIR {
            return Err(EISDIR);
        }
        self.parent_allows_mutation(path)?;
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

    fn mkdir(&mut self, path: Vec<u8>, mode: u32) -> Result<(), i32> {
        if self.find_node(&path).is_some() {
            return Err(EEXIST);
        }
        self.parent_allows_mutation(&path)?;
        self.allocate_node(Some(path), S_IFDIR | (mode & 0o777), true)?;
        Ok(())
    }

    fn rmdir(&mut self, path: &[u8]) -> Result<(), i32> {
        if path == b"/" {
            return Err(EBUSY);
        }
        let index = self.find_node(path).ok_or(ENOENT)?;
        if self.node_mode(index).ok_or(ENOENT)? & S_IFMT != S_IFDIR {
            return Err(ENOTDIR);
        }
        self.parent_allows_mutation(path)?;
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
        let (inode, mode, size, links) = if index == ROOT_NODE {
            (1, S_IFDIR | 0o755, 0, 1)
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
            )
        };
        Ok(CStat {
            device: 1,
            inode,
            links,
            mode,
            user: 0,
            group: 0,
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
}

static FILESYSTEM: IrqSpinLock<FileSystem> = IrqSpinLock::new(FileSystem::new());

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
        return 0;
    }
    if file_descriptor == 1 || file_descriptor == 2 {
        return if events & POLLOUT != 0 { POLLOUT } else { 0 };
    }
    let filesystem = FILESYSTEM.lock();
    let Ok(slot) = filesystem.descriptor_slot(file_descriptor) else {
        return POLLNVAL;
    };
    let Some(descriptor) = filesystem.descriptors[slot] else {
        return POLLNVAL;
    };
    let mut readiness = 0;
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
    if input.is_empty() {
        return Err(ENOENT);
    }
    let reserve = input.len().checked_add(1).ok_or(ENAMETOOLONG)?;
    if reserve > PATH_MAX + 1 {
        return Err(ENAMETOOLONG);
    }
    let mut output = Vec::new();
    output.try_reserve(reserve).map_err(|_| ENOMEM)?;
    output.push(b'/');
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

#[no_mangle]
pub unsafe extern "C" fn kz_rust_open(path: *const c_char, flags: i32, mode: u32) -> i32 {
    with_path(path, |path| {
        if path.as_slice() == b"/dev/net/tun" {
            if flags & O_ACCMODE != O_RDWR
                || flags & !(O_ACCMODE | O_NONBLOCK | O_CLOEXEC | O_NOCTTY) != 0
            {
                return Err(EINVAL);
            }
            return network::open_tun_device(flags & O_NONBLOCK != 0)
                .map_err(|error| -(error as i32));
        }
        FILESYSTEM.lock().open(path, flags, mode)
    })
}

#[no_mangle]
pub unsafe extern "C" fn read(file_descriptor: i32, buffer: *mut c_void, length: usize) -> isize {
    if buffer.is_null() && length != 0 {
        set_errno(EFAULT);
        return -1;
    }
    if length > isize::MAX as usize {
        set_errno(EINVAL);
        return -1;
    }
    if file_descriptor == 0 {
        return 0;
    }
    if length == 0 {
        return 0;
    }
    if network::is_socket_fd(file_descriptor) {
        let output = unsafe { core::slice::from_raw_parts_mut(buffer.cast::<u8>(), length) };
        return match network::receive(file_descriptor, output) {
            Ok(amount) => amount as isize,
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
                Ok(amount) => return amount as isize,
                Err(KernelError::Again) => match network::packet_nonblocking(file_descriptor) {
                    Ok(true) => {
                        set_errno(-(KernelError::Again as i32));
                        return -1;
                    }
                    Ok(false) => {
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
    if file_descriptor < FIRST_DESCRIPTOR {
        set_errno(EBADF);
        return -1;
    }
    match FILESYSTEM
        .lock()
        .read(file_descriptor, buffer.cast::<u8>(), length)
    {
        Ok(amount) => amount as isize,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn write(
    file_descriptor: i32,
    buffer: *const c_void,
    length: usize,
) -> isize {
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
        let mut written = 0;
        while written < length {
            let chunk = (length - written).min(512);
            let result = unsafe {
                crate::kernel::runtime::runtime_ffi::kz_host_console_write(
                    buffer.cast::<u8>().add(written),
                    chunk,
                )
            };
            if result <= 0 {
                if written == 0 {
                    set_errno(5);
                    return -1;
                }
                break;
            }
            written += result as usize;
        }
        return written as isize;
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
    if file_descriptor < FIRST_DESCRIPTOR {
        set_errno(EBADF);
        return -1;
    }
    match FILESYSTEM
        .lock()
        .write(file_descriptor, buffer.cast::<u8>(), length)
    {
        Ok(amount) => amount as isize,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn close(file_descriptor: i32) -> i32 {
    if (0..=2).contains(&file_descriptor) {
        return 0;
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
    match FILESYSTEM.lock().close(file_descriptor) {
        Ok(()) => 0,
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
    match FILESYSTEM.lock().unlink(&normalized) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn mkdir(path: *const c_char, mode: u32) -> i32 {
    with_path(path, |path| FILESYSTEM.lock().mkdir(path, mode).map(|()| 0))
}

#[no_mangle]
pub unsafe extern "C" fn rmdir(path: *const c_char) -> i32 {
    let normalized = match unsafe { normalize_path(path) } {
        Ok(path) => path,
        Err(error) => return fail(error),
    };
    match FILESYSTEM.lock().rmdir(&normalized) {
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
        slot.and_then(|slot| {
            let descriptor = filesystem.descriptors[slot].ok_or(EBADF)?;
            filesystem.stat_node(descriptor.node)
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
    let status = {
        let filesystem = FILESYSTEM.lock();
        filesystem
            .find_node(&normalized)
            .ok_or(ENOENT)
            .and_then(|index| filesystem.stat_node(index))
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
    let mut filesystem = FILESYSTEM.lock();
    let slot = filesystem
        .descriptors
        .iter()
        .position(Option::is_none)
        .ok_or(EMFILE)?;
    let node = filesystem.allocate_node(None, S_IFREG | 0o600, false)?;
    filesystem.nodes[node].as_mut().ok_or(ENOMEM)?.open_count = 1;
    filesystem.descriptors[slot] = Some(Descriptor {
        node,
        offset: 0,
        readable: true,
        writable: true,
        append: false,
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
    -(error as i32)
}

fn metadata_for_path(path: *const c_char) -> Result<NodeMetadata, i32> {
    let path = unsafe { normalize_path(path) }?;
    let mut filesystem = FILESYSTEM.lock();
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
    let name = unsafe { xattr_name(name) }?;
    let value = unsafe { xattr_value(value, size) }?;
    metadata
        .set_xattr(&name, &value, flags)
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
    let name = unsafe { xattr_name(name) }?;
    let value = unsafe { xattr_value(value, size) }?;
    metadata
        .set_xattr(&name, &value, flags)
        .map_err(metadata_errno)
}

fn remove_xattr_path(path: *const c_char, name: *const c_char) -> Result<(), i32> {
    let metadata = metadata_for_path(path)?;
    let name = unsafe { xattr_name(name) }?;
    metadata.remove_xattr(&name).map_err(metadata_errno)
}

fn remove_xattr_fd(file_descriptor: i32, name: *const c_char) -> Result<(), i32> {
    let metadata = metadata_for_fd(file_descriptor)?;
    let name = unsafe { xattr_name(name) }?;
    metadata.remove_xattr(&name).map_err(metadata_errno)
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
    match metadata.set_flags(flags).map_err(metadata_errno) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}
