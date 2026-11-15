/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::string::String;

use super::super::SyscallFrame;
use super::path::{copy_user_path, resolve_at_directory};
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::vfs::{
    FileHandleApi, Node, NodeApi as _, NodeMetadata, NodeMetadataApi as _, VfsApi as _,
};

const AT_FDCWD: i64 = -100;
const AT_SYMLINK_NOFOLLOW: u64 = 0x100;
const AT_NO_AUTOMOUNT: u64 = 0x800;
const AT_EMPTY_PATH: u64 = 0x1000;
const AT_SUPPORTED_FLAGS: u64 = AT_SYMLINK_NOFOLLOW | AT_NO_AUTOMOUNT | AT_EMPTY_PATH;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;
const BLOCK_SIZE: u64 = 512;
const PREFERRED_BLOCK_SIZE: u64 = 4096;

#[derive(Clone, Copy)]
struct StatData {
    device: u64,
    inode: u64,
    mode: u32,
    link_count: u32,
    owner: u32,
    group: u32,
    size: u64,
}

#[derive(Clone, Copy)]
struct StatLayout {
    length: usize,
    device: usize,
    inode: usize,
    link_count: usize,
    link_count_width: usize,
    mode: usize,
    owner: usize,
    group: usize,
    special_device: usize,
    size: usize,
    block_size: usize,
    block_size_width: usize,
    blocks: usize,
    access_time: usize,
    access_time_nsec: usize,
    modification_time: usize,
    modification_time_nsec: usize,
    change_time: usize,
    change_time_nsec: usize,
}

#[cfg(target_arch = "x86_64")]
const STAT_LAYOUT: StatLayout = StatLayout {
    length: 144,
    device: 0,
    inode: 8,
    link_count: 16,
    link_count_width: 8,
    mode: 24,
    owner: 28,
    group: 32,
    special_device: 40,
    size: 48,
    block_size: 56,
    block_size_width: 8,
    blocks: 64,
    access_time: 72,
    access_time_nsec: 80,
    modification_time: 88,
    modification_time_nsec: 96,
    change_time: 104,
    change_time_nsec: 112,
};

#[cfg(target_arch = "aarch64")]
const STAT_LAYOUT: StatLayout = StatLayout {
    length: 128,
    device: 0,
    inode: 8,
    link_count: 20,
    link_count_width: 4,
    mode: 16,
    owner: 24,
    group: 28,
    special_device: 32,
    size: 48,
    block_size: 56,
    block_size_width: 4,
    blocks: 64,
    access_time: 72,
    access_time_nsec: 80,
    modification_time: 88,
    modification_time_nsec: 96,
    change_time: 104,
    change_time_nsec: 112,
};

pub(crate) struct Fstat;
pub(crate) struct Newfstatat;
#[cfg(target_arch = "x86_64")]
pub(crate) struct Stat;
#[cfg(target_arch = "x86_64")]
pub(crate) struct Lstat;

impl Fstat {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Fstat;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let process = ThreadManager::current_process()?;
        let data = stat_from_descriptor(&process, frame.arguments[0])?;
        write_stat(&process, frame.arguments[1], data)
    }
}

impl Newfstatat {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Newfstatat;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let directory_file_descriptor = frame.arguments[0];
        let output_address = frame.arguments[2];
        let flags = frame.arguments[3];
        if flags & !AT_SUPPORTED_FLAGS != 0 {
            return Err(Error::Inval);
        }
        let process = ThreadManager::current_process()?;
        let address_space = process.address_space();
        let user_path = copy_user_path(
            &address_space,
            usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?,
        )?;
        if user_path.is_empty() {
            if flags & AT_EMPTY_PATH == 0 {
                return Err(Error::NoEnt);
            }
            if directory_file_descriptor as i64 != AT_FDCWD {
                let data = stat_from_descriptor(&process, directory_file_descriptor)?;
                return write_stat(&process, output_address, data);
            }
        }

        let path = if user_path.is_empty() {
            let current_directory = process.current_directory()?;
            String::from_utf8(current_directory).map_err(|_| Error::Inval)?
        } else {
            resolve_at_directory(user_path, directory_file_descriptor, &process)?
        };
        let vfs = crate::kernel::vfs::mounted().ok_or(Error::NoDev)?;
        let credentials = process.credentials()?;
        let resolved = if flags & AT_SYMLINK_NOFOLLOW != 0 {
            vfs.resolve_with_path_no_follow_blocking(&path, &credentials)
        } else {
            vfs.resolve_with_path_blocking(&path, &credentials)
        }?;
        let data = stat_from_node(&resolved.0)?;
        write_stat(&process, output_address, data)
    }
}

#[cfg(target_arch = "x86_64")]
impl Stat {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Stat;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        path_stat(frame, 0)
    }
}

#[cfg(target_arch = "x86_64")]
impl Lstat {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Lstat;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        path_stat(frame, AT_SYMLINK_NOFOLLOW)
    }
}

#[cfg(target_arch = "x86_64")]
fn path_stat(mut frame: SyscallFrame, flags: u64) -> Result<i64, Error> {
    let path_address = frame.arguments[0];
    let output_address = frame.arguments[1];
    frame.arguments[0] = AT_FDCWD as u64;
    frame.arguments[1] = path_address;
    frame.arguments[2] = output_address;
    frame.arguments[3] = flags;
    Newfstatat::handle(frame)
}

fn stat_from_descriptor(process: &Process, descriptor: u64) -> Result<StatData, Error> {
    let descriptor = u32::try_from(descriptor).map_err(|_| Error::Badf)?;
    let object = process.file_descriptor(descriptor)?;
    let file = object.take_file()?;
    let result = stat_from_file(&file);
    object.restore_file(file)?;
    result
}

fn stat_from_file(file: &impl FileHandleApi) -> Result<StatData, Error> {
    let metadata = file.metadata();
    let file_type = if file.is_directory() { S_IFDIR } else { S_IFREG };
    let size = match file.size_blocking() {
        Ok(size) => size,
        Err(Error::IsDir | Error::NoSys | Error::Inval) => 0,
        Err(error) => return Err(error),
    };
    Ok(stat_data(&metadata, file_type, size))
}

fn stat_from_node(node: &Node) -> Result<StatData, Error> {
    let metadata = node.metadata();
    let (file_type, size) = match node {
        Node::Directory { .. } => (S_IFDIR, 0),
        Node::File { .. } => {
            let size = match node.size_blocking() {
                Ok(size) => size,
                Err(Error::NoSys | Error::IsDir | Error::Inval) => 0,
                Err(error) => return Err(error),
            };
            (S_IFREG, size)
        }
        Node::Symlink { target, .. } => {
            (S_IFLNK, u64::try_from(target.len()).map_err(|_| Error::Overflow)?)
        }
    };
    Ok(stat_data(&metadata, file_type, size))
}

fn stat_data(metadata: &NodeMetadata, file_type: u32, size: u64) -> StatData {
    StatData {
        device: 0,
        inode: metadata.identity(),
        mode: file_type | u32::from(metadata.mode()),
        link_count: metadata.link_count(),
        owner: metadata.owner().raw(),
        group: metadata.group().raw(),
        size,
    }
}

fn write_stat(process: &Process, output_address: u64, data: StatData) -> Result<i64, Error> {
    let output_address = usize::try_from(output_address).map_err(|_| Error::Fault)?;
    let bytes = encode_stat(data)?;
    let mut address_space = process.address_space();
    address_space
        .write(output_address, &bytes)
        .map_err(|_| Error::Fault)?;
    Ok(0)
}

fn encode_stat(data: StatData) -> Result<[u8; STAT_LAYOUT.length], Error> {
    let mut output = [0; STAT_LAYOUT.length];
    write_unsigned(&mut output, STAT_LAYOUT.device, 8, data.device)?;
    write_unsigned(&mut output, STAT_LAYOUT.inode, 8, data.inode)?;
    write_unsigned(
        &mut output,
        STAT_LAYOUT.link_count,
        STAT_LAYOUT.link_count_width,
        u64::from(data.link_count),
    )?;
    write_unsigned(&mut output, STAT_LAYOUT.mode, 4, u64::from(data.mode))?;
    write_unsigned(&mut output, STAT_LAYOUT.owner, 4, u64::from(data.owner))?;
    write_unsigned(&mut output, STAT_LAYOUT.group, 4, u64::from(data.group))?;
    write_unsigned(&mut output, STAT_LAYOUT.special_device, 8, 0)?;
    write_unsigned(
        &mut output,
        STAT_LAYOUT.size,
        8,
        i64::try_from(data.size).map_err(|_| Error::Overflow)? as u64,
    )?;
    write_unsigned(
        &mut output,
        STAT_LAYOUT.block_size,
        STAT_LAYOUT.block_size_width,
        PREFERRED_BLOCK_SIZE,
    )?;
    write_unsigned(
        &mut output,
        STAT_LAYOUT.blocks,
        8,
        data.size.saturating_add(BLOCK_SIZE - 1) / BLOCK_SIZE,
    )?;
    for offset in [
        STAT_LAYOUT.access_time,
        STAT_LAYOUT.access_time_nsec,
        STAT_LAYOUT.modification_time,
        STAT_LAYOUT.modification_time_nsec,
        STAT_LAYOUT.change_time,
        STAT_LAYOUT.change_time_nsec,
    ] {
        write_unsigned(&mut output, offset, 8, 0)?;
    }
    Ok(output)
}

fn write_unsigned(output: &mut [u8], offset: usize, width: usize, value: u64) -> Result<(), Error> {
    let end = offset.checked_add(width).ok_or(Error::Overflow)?;
    let field = output.get_mut(offset..end).ok_or(Error::Inval)?;
    match width {
        4 => field.copy_from_slice(
            &u32::try_from(value)
                .map_err(|_| Error::Overflow)?
                .to_le_bytes(),
        ),
        8 => field.copy_from_slice(&value.to_le_bytes()),
        _ => return Err(Error::Inval),
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/stat.rs"]
mod tests;
