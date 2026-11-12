/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use of this file is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::kernel::driver::virtio::queue::{Descriptor, DmaQueue};
use crate::kernel::driver::virtio::transport::VirtioTransport;
use crate::kernel::error::Error;
use crate::kernel::memory::dma::{DmaArena, DmaBuffer};
use crate::kernel::process::{GroupId, UserId};
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::vfs::{
    AppendResult, DirectoryEntry, DirectoryPlugin, FilePlugin, Node, NodeMetadata, VfsFuture,
    VfsResult,
};

struct FuseDirectory {
    filesystem: Arc<VirtioFs>,
    node_id: u64,
}

struct FuseFile {
    filesystem: Arc<VirtioFs>,
    node_id: u64,
}

pub(crate) struct VirtioFs {
    transport: VirtioTransport,
    _high_priority_queue: IrqSpinLock<DmaQueue<{ Self::QUEUE_SIZE }>>,
    request_state: IrqSpinLock<RequestState>,
    node_metadata: IrqSpinLock<BTreeMap<u64, NodeMetadata>>,
    write_locks: IrqSpinLock<BTreeMap<u64, Weak<AtomicBool>>>,
    next_unique: AtomicU64,
    tag: String,
    max_write: usize,
}

struct RequestState {
    queue: DmaQueue<{ VirtioFs::QUEUE_SIZE }>,
    message_buffer: DmaBuffer,
    in_flight_head: Option<u16>,
    orphaned: bool,
}

struct InFlightRequest<'a> {
    filesystem: &'a VirtioFs,
    head: u16,
    completed: bool,
}

struct FileWriteGuard<'a>(&'a AtomicBool);

impl Drop for FileWriteGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

async fn acquire_file_write_lock(lock: &AtomicBool) -> FileWriteGuard<'_> {
    while lock
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        crate::kernel::scheduler::future::yield_now().await;
    }
    FileWriteGuard(lock)
}

impl Drop for InFlightRequest<'_> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let mut state = self.filesystem.request_state.lock();
        if state.in_flight_head == Some(self.head) {
            state.orphaned = true;
        }
    }
}

impl VirtioFs {
    const DEVICE_ID: u32 = 26;
    const QUEUE_SIZE: usize = 8;
    const HIPRIO_QUEUE: u32 = 0;
    const REQUEST_QUEUE: u32 = 2;
    const TAG_SIZE: usize = 36;
    const CONFIG_REQUEST_QUEUE_COUNT_OFFSET: usize = 36;
    const MAX_REQUEST_SIZE: usize = 32 * 1024;
    const MAX_RESPONSE_SIZE: usize = 32 * 1024;
    const MESSAGE_BUFFER_SIZE: usize = Self::MAX_REQUEST_SIZE + Self::MAX_RESPONSE_SIZE;
    const FUSE_IN_HEADER_SIZE: usize = 40;
    const FUSE_OUT_HEADER_SIZE: usize = 16;
    const FUSE_ROOT_ID: u64 = 1;
    const FUSE_KERNEL_MAJOR: u32 = 7;
    const FUSE_KERNEL_MINOR: u32 = 41;
    const FUSE_INIT: u32 = 26;
    const FUSE_LOOKUP: u32 = 1;
    const FUSE_GETATTR: u32 = 3;
    const FUSE_OPEN: u32 = 14;
    const FUSE_READ: u32 = 15;
    const FUSE_WRITE: u32 = 16;
    const FUSE_RELEASE: u32 = 18;
    const FUSE_OPENDIR: u32 = 27;
    const FUSE_READDIR: u32 = 28;
    const FUSE_RELEASEDIR: u32 = 29;
    const FUSE_ENTRY_OUT_SIZE: usize = 128;
    const FUSE_ENTRY_MODE_OFFSET: usize = 100;
    const FUSE_ENTRY_UID_OFFSET: usize = 108;
    const FUSE_ENTRY_GID_OFFSET: usize = 112;
    const FUSE_GETATTR_IN_SIZE: usize = 16;
    const FUSE_ATTR_OUT_SIZE: usize = 104;
    const FUSE_ATTR_MODE_OFFSET: usize = 76;
    const FUSE_ATTR_UID_OFFSET: usize = 84;
    const FUSE_ATTR_GID_OFFSET: usize = 88;
    const FUSE_OPEN_OUT_SIZE: usize = 16;
    const FUSE_READ_IN_SIZE: usize = 40;
    const FUSE_WRITE_OUT_SIZE: usize = 8;
    const FUSE_DIRENT_HEADER_SIZE: usize = 24;
    const FUSE_MIN_READ_BUFFER: usize = 8192;
    const DIRECTORY_TYPE: u32 = 4;
    const REGULAR_FILE_TYPE: u32 = 8;
    const MODE_TYPE_MASK: u32 = 0o170000;
    const MODE_DIRECTORY: u32 = 0o040000;
    const MODE_REGULAR_FILE: u32 = 0o100000;
    const COMMAND_SPIN_LIMIT: usize = 2_000_000;

    pub(crate) async fn initialize() -> Option<Arc<Self>> {
        let candidate = VirtioTransport::find(Self::DEVICE_ID)?;
        let transport = candidate.initialize(Self::DEVICE_ID).ok()?;
        let request_queue_count =
            transport.read_device_config(Self::CONFIG_REQUEST_QUEUE_COUNT_OFFSET)?;
        if request_queue_count == 0 {
            return None;
        }

        let mut high_priority_queue: DmaQueue<{ Self::QUEUE_SIZE }> = DmaQueue::new()?;
        let mut request_queue: DmaQueue<{ Self::QUEUE_SIZE }> = DmaQueue::new()?;
        let message_buffer = DmaArena::allocate(Self::MESSAGE_BUFFER_SIZE, 16)?;
        high_priority_queue
            .configure(&transport, Self::HIPRIO_QUEUE)
            .ok()?;
        request_queue
            .configure_with_min_size(&transport, Self::REQUEST_QUEUE, 2)
            .ok()?;
        if !transport.activate() {
            return None;
        }

        let mut tag_bytes = [0; Self::TAG_SIZE];
        for (offset, byte) in tag_bytes.iter_mut().enumerate() {
            *byte = transport.read_device_config_byte(offset)?;
        }
        let tag_length = tag_bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(tag_bytes.len());
        let tag = String::from(core::str::from_utf8(&tag_bytes[..tag_length]).ok()?);
        let mut filesystem = Self {
            transport,
            _high_priority_queue: IrqSpinLock::new(high_priority_queue),
            request_state: IrqSpinLock::new(RequestState {
                queue: request_queue,
                message_buffer,
                in_flight_head: None,
                orphaned: false,
            }),
            node_metadata: IrqSpinLock::new(BTreeMap::new()),
            write_locks: IrqSpinLock::new(BTreeMap::new()),
            next_unique: AtomicU64::new(1),
            tag,
            max_write: Self::MAX_REQUEST_SIZE - Self::FUSE_IN_HEADER_SIZE,
        };
        filesystem.initialize_fuse().await.ok()?;
        filesystem.refresh_metadata(Self::FUSE_ROOT_ID).await.ok()?;
        Some(Arc::new(filesystem))
    }

    pub(crate) fn tag(&self) -> &str {
        &self.tag
    }

    fn write_lock(&self, node_id: u64) -> Arc<AtomicBool> {
        let mut write_locks = self.write_locks.lock();
        if let Some(lock) = write_locks.get(&node_id).and_then(|lock| lock.upgrade()) {
            return lock;
        }
        write_locks.retain(|_, lock| lock.strong_count() != 0);
        let lock = Arc::new(AtomicBool::new(false));
        write_locks.insert(node_id, Arc::downgrade(&lock));
        lock
    }

    pub(crate) fn root(self: &Arc<Self>) -> Node {
        Node::directory_with_metadata(
            FuseDirectory {
                filesystem: self.clone(),
                node_id: Self::FUSE_ROOT_ID,
            },
            self.metadata_for_node(Self::FUSE_ROOT_ID),
        )
    }

    fn metadata_for_node(&self, node_id: u64) -> NodeMetadata {
        if let Some(metadata) = self.node_metadata.lock().get(&node_id).cloned() {
            return metadata;
        }
        let metadata = NodeMetadata::new();
        self.node_metadata
            .lock()
            .entry(node_id)
            .or_insert(metadata)
            .clone()
    }

    fn update_metadata_for_node(
        &self,
        node_id: u64,
        owner: u32,
        group: u32,
        mode: u32,
    ) -> VfsResult<NodeMetadata> {
        let metadata = self.metadata_for_node(node_id);
        metadata.update_from_filesystem(
            UserId::new(owner),
            GroupId::new(group),
            (mode & 0o777) as u16,
        )?;
        Ok(metadata)
    }

    async fn refresh_metadata(&self, node_id: u64) -> VfsResult<NodeMetadata> {
        let request = [0; Self::FUSE_GETATTR_IN_SIZE];
        let mut response = [0; Self::FUSE_ATTR_OUT_SIZE];
        let response_length = self
            .transact(Self::FUSE_GETATTR, node_id, &request, &mut response)
            .await?;
        if response_length < Self::FUSE_ATTR_OUT_SIZE {
            return Err(Error::Proto);
        }
        let mode = Self::read_u32(&response, Self::FUSE_ATTR_MODE_OFFSET)?;
        let owner = Self::read_u32(&response, Self::FUSE_ATTR_UID_OFFSET)?;
        let group = Self::read_u32(&response, Self::FUSE_ATTR_GID_OFFSET)?;
        self.update_metadata_for_node(node_id, owner, group, mode)
    }

    async fn initialize_fuse(&mut self) -> Result<(), Error> {
        let mut request = [0; 64];
        Self::write_u32(&mut request, 0, Self::FUSE_KERNEL_MAJOR);
        Self::write_u32(&mut request, 4, Self::FUSE_KERNEL_MINOR);
        let mut response = [0; 64];
        let response_length = self
            .transact(Self::FUSE_INIT, 0, &request, &mut response)
            .await?;
        if response_length < 8 || Self::read_u32(&response, 0)? != Self::FUSE_KERNEL_MAJOR {
            return Err(Error::Proto);
        }
        let minor = Self::read_u32(&response, 4)?;
        self.max_write = if minor >= 5 && response_length >= 24 {
            (Self::read_u32(&response, 20)? as usize)
                .min(Self::MAX_REQUEST_SIZE - Self::FUSE_IN_HEADER_SIZE)
        } else {
            Self::FUSE_MIN_READ_BUFFER
        };
        if self.max_write == 0 {
            return Err(Error::Proto);
        }
        Ok(())
    }

    async fn lookup(self: &Arc<Self>, parent: u64, name: &str) -> VfsResult<Node> {
        if name.is_empty() || name.contains('/') || name.as_bytes().contains(&0) {
            return Err(Error::Inval);
        }
        let mut payload = Vec::with_capacity(name.len() + 1);
        payload.extend_from_slice(name.as_bytes());
        payload.push(0);
        let mut response = [0; Self::FUSE_ENTRY_OUT_SIZE];
        let response_length = self
            .transact(Self::FUSE_LOOKUP, parent, &payload, &mut response)
            .await?;
        if response_length < Self::FUSE_ENTRY_OUT_SIZE {
            return Err(Error::Proto);
        }
        let node_id = Self::read_u64(&response, 0)?;
        let mode = Self::read_u32(&response, Self::FUSE_ENTRY_MODE_OFFSET)?;
        let owner = Self::read_u32(&response, Self::FUSE_ENTRY_UID_OFFSET)?;
        let group = Self::read_u32(&response, Self::FUSE_ENTRY_GID_OFFSET)?;
        self.node_from_type(node_id, mode, owner, group)
    }

    async fn read_dir(self: &Arc<Self>, node_id: u64) -> VfsResult<Vec<DirectoryEntry>> {
        let handle = self.open(node_id, Self::FUSE_OPENDIR, 0).await?;
        let result = self.read_dir_handle(node_id, handle).await;
        let release_result = self
            .release(node_id, handle, Self::FUSE_RELEASEDIR)
            .await;
        match result {
            Ok(entries) => {
                release_result?;
                Ok(entries)
            }
            Err(error) => {
                let _ = release_result;
                Err(error)
            }
        }
    }

    async fn read_dir_handle(
        self: &Arc<Self>,
        node_id: u64,
        handle: u64,
    ) -> VfsResult<Vec<DirectoryEntry>> {
        let mut entries = Vec::new();
        let mut response = vec![0; Self::MAX_RESPONSE_SIZE - Self::FUSE_OUT_HEADER_SIZE];
        let mut offset = 0u64;
        loop {
            let mut payload = [0; Self::FUSE_READ_IN_SIZE];
            Self::write_u64(&mut payload, 0, handle);
            Self::write_u64(&mut payload, 8, offset);
            Self::write_u32(&mut payload, 16, response.len() as u32);
            let length = self
                .transact(Self::FUSE_READDIR, node_id, &payload, &mut response)
                .await?;
            if length == 0 {
                break;
            }
            let previous_offset = offset;
            let mut cursor = 0usize;
            while cursor < length {
                if length - cursor < Self::FUSE_DIRENT_HEADER_SIZE {
                    return Err(Error::Proto);
                }
                let inode = Self::read_u64(&response, cursor)?;
                let next_offset = Self::read_u64(&response, cursor + 8)?;
                let name_length = Self::read_u32(&response, cursor + 16)? as usize;
                let file_type = Self::read_u32(&response, cursor + 20)?;
                let record_length = Self::align_dirent(
                    Self::FUSE_DIRENT_HEADER_SIZE
                        .checked_add(name_length)
                        .ok_or(Error::Proto)?,
                );
                if record_length > length - cursor {
                    return Err(Error::Proto);
                }
                let name_bytes = &response[cursor + Self::FUSE_DIRENT_HEADER_SIZE
                    ..cursor + Self::FUSE_DIRENT_HEADER_SIZE + name_length];
                if name_bytes != b"." && name_bytes != b".." {
                    let name =
                        String::from(core::str::from_utf8(name_bytes).map_err(|_| Error::Ilseq)?);
                    let metadata = match file_type {
                        Self::DIRECTORY_TYPE | Self::REGULAR_FILE_TYPE => {
                            self.refresh_metadata(inode).await?
                        }
                        _ => {
                            cursor += record_length;
                            offset = next_offset;
                            continue;
                        }
                    };
                    let node = match file_type {
                        Self::DIRECTORY_TYPE => Node::directory_with_metadata(
                            FuseDirectory {
                                filesystem: self.clone(),
                                node_id: inode,
                            },
                            metadata,
                        ),
                        Self::REGULAR_FILE_TYPE => Node::file_with_metadata(
                            FuseFile {
                                filesystem: self.clone(),
                                node_id: inode,
                            },
                            metadata,
                        ),
                        _ => {
                            cursor += record_length;
                            offset = next_offset;
                            continue;
                        }
                    };
                    entries.push(DirectoryEntry { name, node });
                }
                cursor += record_length;
                offset = next_offset;
            }
            if offset == previous_offset || length < response.len() {
                break;
            }
        }
        Ok(entries)
    }

    async fn file_size(&self, node_id: u64) -> VfsResult<u64> {
        let request = [0; Self::FUSE_GETATTR_IN_SIZE];
        let mut response = [0; Self::FUSE_ATTR_OUT_SIZE];
        let length = self
            .transact(Self::FUSE_GETATTR, node_id, &request, &mut response)
            .await?;
        if length < Self::FUSE_ATTR_OUT_SIZE {
            return Err(Error::Proto);
        }
        Self::read_u64(&response, 24)
    }

    async fn read_at(&self, node_id: u64, offset: u64, buffer: &mut [u8]) -> VfsResult<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let handle = self.open(node_id, Self::FUSE_OPEN, 0).await?;
        let size = buffer.len().min(self.max_write);
        let mut payload = [0; Self::FUSE_READ_IN_SIZE];
        Self::write_u64(&mut payload, 0, handle);
        Self::write_u64(&mut payload, 8, offset);
        Self::write_u32(&mut payload, 16, size as u32);
        let result = self
            .transact(Self::FUSE_READ, node_id, &payload, &mut buffer[..size])
            .await;
        let release_result = self.release(node_id, handle, Self::FUSE_RELEASE).await;
        match result {
            Ok(length) => {
                release_result?;
                Ok(length)
            }
            Err(error) => {
                let _ = release_result;
                Err(error)
            }
        }
    }

    async fn write_at(&self, node_id: u64, offset: u64, buffer: &[u8]) -> VfsResult<usize> {
        let lock = self.write_lock(node_id);
        let _guard = acquire_file_write_lock(&lock).await;
        self.write_at_unlocked(node_id, offset, buffer).await
    }

    async fn append(&self, node_id: u64, buffer: &[u8]) -> VfsResult<AppendResult> {
        let lock = self.write_lock(node_id);
        let _guard = acquire_file_write_lock(&lock).await;
        let offset = self.file_size(node_id).await?;
        let requested = u64::try_from(buffer.len()).map_err(|_| Error::Fbig)?;
        offset.checked_add(requested).ok_or(Error::Fbig)?;
        let written = self.write_at_unlocked(node_id, offset, buffer).await?;
        let end_offset = offset
            .checked_add(u64::try_from(written).map_err(|_| Error::Fbig)?)
            .ok_or(Error::Fbig)?;
        Ok(AppendResult {
            end_offset,
            written,
        })
    }

    async fn write_at_unlocked(
        &self,
        node_id: u64,
        offset: u64,
        buffer: &[u8],
    ) -> VfsResult<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let handle = self.open(node_id, Self::FUSE_OPEN, 1).await?;
        let size = buffer
            .len()
            .min(self.max_write)
            .min(Self::MAX_REQUEST_SIZE - Self::FUSE_IN_HEADER_SIZE - Self::FUSE_READ_IN_SIZE);
        let mut payload = vec![0; Self::FUSE_READ_IN_SIZE + size];
        Self::write_u64(&mut payload, 0, handle);
        Self::write_u64(&mut payload, 8, offset);
        Self::write_u32(&mut payload, 16, size as u32);
        payload[Self::FUSE_READ_IN_SIZE..].copy_from_slice(&buffer[..size]);
        let mut response = [0; Self::FUSE_WRITE_OUT_SIZE];
        let result = self
            .transact(Self::FUSE_WRITE, node_id, &payload, &mut response)
            .await;
        let release_result = self.release(node_id, handle, Self::FUSE_RELEASE).await;
        match result {
            Ok(length) if length >= Self::FUSE_WRITE_OUT_SIZE => {
                release_result?;
                let written = Self::read_u32(&response, 0)? as usize;
                if written > size {
                    return Err(Error::Proto);
                }
                Ok(written)
            }
            Ok(_) => Err(Error::Proto),
            Err(error) => {
                let _ = release_result;
                Err(error)
            }
        }
    }

    async fn open(&self, node_id: u64, opcode: u32, flags: u32) -> VfsResult<u64> {
        let mut payload = [0; 8];
        Self::write_u32(&mut payload, 0, flags);
        let mut response = [0; Self::FUSE_OPEN_OUT_SIZE];
        let length = self.transact(opcode, node_id, &payload, &mut response).await?;
        if length < Self::FUSE_OPEN_OUT_SIZE {
            return Err(Error::Proto);
        }
        Self::read_u64(&response, 0)
    }

    async fn release(&self, node_id: u64, handle: u64, opcode: u32) -> VfsResult<()> {
        let mut payload = [0; 24];
        Self::write_u64(&mut payload, 0, handle);
        self.transact(opcode, node_id, &payload, &mut []).await?;
        Ok(())
    }

    async fn transact(
        &self,
        opcode: u32,
        node_id: u64,
        payload: &[u8],
        response: &mut [u8],
    ) -> VfsResult<usize> {
        let request_length = Self::FUSE_IN_HEADER_SIZE
            .checked_add(payload.len())
            .ok_or(Error::MsgSize)?;
        let response_descriptor_length = Self::FUSE_OUT_HEADER_SIZE
            .checked_add(response.len())
            .ok_or(Error::MsgSize)?;
        if request_length > Self::MAX_REQUEST_SIZE
            || response_descriptor_length > Self::MAX_RESPONSE_SIZE
        {
            return Err(Error::MsgSize);
        }

        let unique = self.next_unique.fetch_add(1, Ordering::Relaxed);
        let head = {
            let mut state = self.request_state.lock();
            if let Some(orphaned_head) = state.in_flight_head {
                if !state.orphaned {
                    return Err(Error::Busy);
                }
                let Some(used) = state.queue.poll_used() else {
                    return Err(Error::Busy);
                };
                if used.id != u32::from(orphaned_head) {
                    return Err(Error::Proto);
                }
                state.in_flight_head = None;
                state.orphaned = false;
            }

            let base = state.message_buffer.bus_address();
            let bytes = state.message_buffer.as_mut_slice();
            bytes[..Self::MAX_REQUEST_SIZE].fill(0);
            Self::write_u32(bytes, 0, request_length as u32);
            Self::write_u32(bytes, 4, opcode);
            Self::write_u64(bytes, 8, unique);
            Self::write_u64(bytes, 16, node_id);
            bytes[Self::FUSE_IN_HEADER_SIZE..request_length].copy_from_slice(payload);
            bytes[Self::MAX_REQUEST_SIZE..].fill(0);

            let descriptors = [
                Descriptor::new(base, request_length as u32, false),
                Descriptor::new(
                    base + Self::MAX_REQUEST_SIZE as u64,
                    response_descriptor_length as u32,
                    true,
                ),
            ];
            let head = state.queue.submit(&descriptors).ok_or(Error::Busy)?;
            state.in_flight_head = Some(head);
            state.orphaned = false;
            head
        };
        DmaArena::sync_for_device();
        self.transport.notify(Self::REQUEST_QUEUE);

        let mut in_flight = InFlightRequest {
            filesystem: self,
            head,
            completed: false,
        };
        for _ in 0..Self::COMMAND_SPIN_LIMIT {
            let used = {
                let mut state = self.request_state.lock();
                state.queue.poll_used()
            };
            let Some(used) = used else {
                crate::kernel::scheduler::future::yield_now().await;
                continue;
            };
            let body_length = {
                let mut state = self.request_state.lock();
                if state.in_flight_head != Some(head) || used.id != u32::from(head) {
                    return Err(Error::Proto);
                }
                state.in_flight_head = None;
                state.orphaned = false;
                in_flight.completed = true;
                if used.length < Self::FUSE_OUT_HEADER_SIZE as u32 {
                    return Err(Error::Proto);
                }
                let bytes = state.message_buffer.as_mut_slice();
                let response_offset = Self::MAX_REQUEST_SIZE;
                let response_length = Self::read_u32(bytes, response_offset)? as usize;
                let error = Self::read_i32(bytes, response_offset + 4)?;
                let response_unique = Self::read_u64(bytes, response_offset + 8)?;
                if response_unique != unique
                    || response_length < Self::FUSE_OUT_HEADER_SIZE
                    || response_length > used.length as usize
                {
                    return Err(Error::Proto);
                }
                if error != 0 {
                    return Err(Self::map_fuse_error(error));
                }
                let body_length = response_length - Self::FUSE_OUT_HEADER_SIZE;
                if body_length > response.len() {
                    return Err(Error::MsgSize);
                }
                response[..body_length].copy_from_slice(
                    &bytes[response_offset + Self::FUSE_OUT_HEADER_SIZE
                        ..response_offset + response_length],
                );
                body_length
            };
            return Ok(body_length);
        }
        Err(Error::Io)
    }

    fn map_fuse_error(error: i32) -> Error {
        match error {
            -1 => Error::Acces,
            -2 => Error::NoEnt,
            -5 => Error::Io,
            -12 => Error::NoMem,
            -13 => Error::Acces,
            -20 => Error::NotDir,
            -21 => Error::IsDir,
            -22 => Error::Inval,
            -30 => Error::Rofs,
            -38 => Error::NoSys,
            _ => Error::Io,
        }
    }

    fn node_from_type(
        self: &Arc<Self>,
        node_id: u64,
        file_type: u32,
        owner: u32,
        group: u32,
    ) -> VfsResult<Node> {
        let metadata = self.update_metadata_for_node(node_id, owner, group, file_type)?;
        match file_type & Self::MODE_TYPE_MASK {
            Self::MODE_DIRECTORY => Ok(Node::directory_with_metadata(
                FuseDirectory {
                    filesystem: self.clone(),
                    node_id,
                },
                metadata,
            )),
            Self::MODE_REGULAR_FILE => Ok(Node::file_with_metadata(
                FuseFile {
                    filesystem: self.clone(),
                    node_id,
                },
                metadata,
            )),
            _ => Err(Error::OpNotSupp),
        }
    }

    fn align_dirent(length: usize) -> usize {
        (length + 7) & !7
    }

    fn write_u32(buffer: &mut [u8], offset: usize, value: u32) {
        buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u64(buffer: &mut [u8], offset: usize, value: u64) {
        buffer[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn read_u32(buffer: &[u8], offset: usize) -> VfsResult<u32> {
        let bytes = buffer.get(offset..offset + 4).ok_or(Error::Proto)?;
        Ok(u32::from_le_bytes(
            bytes.try_into().map_err(|_| Error::Proto)?,
        ))
    }

    fn read_i32(buffer: &[u8], offset: usize) -> VfsResult<i32> {
        Self::read_u32(buffer, offset).map(|value| value as i32)
    }

    fn read_u64(buffer: &[u8], offset: usize) -> VfsResult<u64> {
        let bytes = buffer.get(offset..offset + 8).ok_or(Error::Proto)?;
        Ok(u64::from_le_bytes(
            bytes.try_into().map_err(|_| Error::Proto)?,
        ))
    }
}

impl DirectoryPlugin for FuseDirectory {
    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, VfsResult<Node>> {
        let filesystem = self.filesystem.clone();
        let parent = self.node_id;
        let name = String::from(name);
        Box::pin(async move {
            if name.is_empty() || name.contains('/') || name.as_bytes().contains(&0) {
                return Err(Error::Inval);
            }
            let mut payload = Vec::with_capacity(name.len() + 1);
            payload.extend_from_slice(name.as_bytes());
            payload.push(0);
            let mut response = [0; VirtioFs::FUSE_ENTRY_OUT_SIZE];
            let length = filesystem
                .transact(VirtioFs::FUSE_LOOKUP, parent, &payload, &mut response)
                .await?;
            if length < VirtioFs::FUSE_ENTRY_OUT_SIZE {
                return Err(Error::Proto);
            }
            let node_id = VirtioFs::read_u64(&response, 0)?;
            let mode = VirtioFs::read_u32(&response, VirtioFs::FUSE_ENTRY_MODE_OFFSET)?;
            let owner = VirtioFs::read_u32(&response, VirtioFs::FUSE_ENTRY_UID_OFFSET)?;
            let group = VirtioFs::read_u32(&response, VirtioFs::FUSE_ENTRY_GID_OFFSET)?;
            filesystem.node_from_type(node_id, mode, owner, group)
        })
    }

    fn read_dir(&self) -> VfsFuture<'_, VfsResult<Vec<DirectoryEntry>>> {
        let filesystem = self.filesystem.clone();
        let node_id = self.node_id;
        Box::pin(async move { filesystem.read_dir(node_id).await })
    }
}

impl FilePlugin for FuseFile {
    fn read_at<'a>(&'a self, offset: u64, buffer: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        let filesystem = self.filesystem.clone();
        let node_id = self.node_id;
        Box::pin(async move { filesystem.read_at(node_id, offset, buffer).await })
    }

    fn size(&self) -> VfsFuture<'_, VfsResult<u64>> {
        let filesystem = self.filesystem.clone();
        let node_id = self.node_id;
        Box::pin(async move { filesystem.file_size(node_id).await })
    }

    fn write_at<'a>(&'a self, offset: u64, buffer: &'a [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        let filesystem = self.filesystem.clone();
        let node_id = self.node_id;
        Box::pin(async move { filesystem.write_at(node_id, offset, buffer).await })
    }

    fn append<'a>(&'a self, buffer: &'a [u8]) -> VfsFuture<'a, VfsResult<AppendResult>> {
        let filesystem = self.filesystem.clone();
        let node_id = self.node_id;
        Box::pin(async move { filesystem.append(node_id, buffer).await })
    }
}
