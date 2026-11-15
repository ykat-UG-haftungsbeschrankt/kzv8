/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use super::super::SyscallFrame;
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::vfs::{
    FileHandleApi as _, Node, NodeApi as _, NodeMetadataApi as _, SeekFrom,
};

const MAX_DIRENT_BUFFER_LENGTH: usize = 1 << 20;
const DIRENT64_HEADER_LENGTH: usize = 19;
const DIRENT64_ALIGNMENT: usize = 8;
const DT_REG: u8 = 8;
const DT_DIR: u8 = 4;
const DT_LNK: u8 = 10;

pub(crate) struct GetDents64;

impl GetDents64 {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::GetDents64;

    pub(crate) fn callback(
        file_descriptor: u64,
        address: u64,
        length: u64,
    ) -> Result<i64, Error> {
        let file_descriptor = u32::try_from(file_descriptor).map_err(|_| Error::Badf)?;
        let length = usize::try_from(length).map_err(|_| Error::Fbig)?;
        if length > MAX_DIRENT_BUFFER_LENGTH {
            return Err(Error::Fbig);
        }
        let address = usize::try_from(address).map_err(|_| Error::Fault)?;
        let process = ThreadManager::current_process()?;
        let file = process.file_descriptor(file_descriptor)?;
        let mut address_space = process.address_space();
        let mut file_handle = file.take_file()?;
        let result = (|| {
            if !file_handle.is_directory() {
                return Err(Error::NotDir);
            }
            let entries = file_handle.read_directory_blocking()?;
            let first_entry = usize::try_from(file_handle.position()).map_err(|_| Error::Overflow)?;
            let mut output = Vec::new();
            output
                .try_reserve_exact(length)
                .map_err(|_| Error::NoMem)?;
            let mut next_entry = first_entry;
            for entry in entries.iter().skip(first_entry) {
                let candidate_next = next_entry.checked_add(1).ok_or(Error::Overflow)?;
                if !append_dirent64(
                    &mut output,
                    length,
                    &entry.name,
                    entry.node.metadata().identity(),
                    candidate_next,
                    dirent_type(&entry.node),
                )? {
                    break;
                }
                next_entry = candidate_next;
            }
            if output.is_empty() {
                return Ok(0);
            }
            address_space
                .write(address, &output)
                .map_err(|_| Error::Fault)?;
            file_handle.seek(SeekFrom::Start(
                u64::try_from(next_entry).map_err(|_| Error::Overflow)?,
            ))?;
            i64::try_from(output.len()).map_err(|_| Error::Overflow)
        })();
        file.restore_file(file_handle)?;
        result
    }

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        Self::callback(
            frame.arguments[0],
            frame.arguments[1],
            frame.arguments[2],
        )
    }
}

fn append_dirent64(
    output: &mut Vec<u8>,
    maximum_length: usize,
    name: &str,
    inode: u64,
    next_entry: usize,
    file_type: u8,
) -> Result<bool, Error> {
    let name = name.as_bytes();
    if name.contains(&0) {
        return Err(Error::Inval);
    }
    let name_length = name.len().checked_add(1).ok_or(Error::Fbig)?;
    let unaligned_length = DIRENT64_HEADER_LENGTH
        .checked_add(name_length)
        .ok_or(Error::Fbig)?;
    let record_length = unaligned_length
        .checked_add(DIRENT64_ALIGNMENT - 1)
        .ok_or(Error::Fbig)?
        & !(DIRENT64_ALIGNMENT - 1);
    let record_length_u16 = u16::try_from(record_length).map_err(|_| Error::NameTooLong)?;
    let next_entry = i64::try_from(next_entry).map_err(|_| Error::Overflow)?;
    let remaining = maximum_length.checked_sub(output.len()).ok_or(Error::Inval)?;
    if record_length > remaining {
        if output.is_empty() {
            return Err(Error::Inval);
        }
        return Ok(false);
    }
    output
        .try_reserve(record_length)
        .map_err(|_| Error::NoMem)?;
    output.extend_from_slice(&inode.to_le_bytes());
    output.extend_from_slice(&next_entry.to_le_bytes());
    output.extend_from_slice(&record_length_u16.to_le_bytes());
    output.push(file_type);
    output.extend_from_slice(name);
    output.push(0);
    output.resize(output.len() + record_length - unaligned_length, 0);
    Ok(true)
}

fn dirent_type(node: &Node) -> u8 {
    match node {
        Node::Directory { .. } => DT_DIR,
        Node::File { .. } => DT_REG,
        Node::Symlink { .. } => DT_LNK,
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/getdents64.rs"]
mod tests;