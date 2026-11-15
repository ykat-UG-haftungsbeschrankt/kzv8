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
use crate::kernel::process::MAX_SUPPLEMENTARY_GROUPS;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct GetGroups;
pub(crate) struct SetGroups;

impl GetGroups {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::GetGroups;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let size = group_buffer_size(frame.arguments[0])?;
        let process = ThreadManager::current_process()?;
        let groups = process.supplementary_group_ids()?;
        if let Some(count) = group_count_result(size, groups.len())? {
            return Ok(count);
        }
        let output = encode_groups(&groups)?;
        if output.is_empty() {
            return Ok(0);
        }
        let address = usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?;
        if address == 0 {
            return Err(Error::Fault);
        }
        let mut address_space = process.address_space();
        address_space
            .write(address, &output)
            .map_err(|_| Error::Fault)?;
        i64::try_from(groups.len()).map_err(|_| Error::Overflow)
    }
}

fn group_buffer_size(value: u64) -> Result<usize, Error> {
    usize::try_from(value as i32).map_err(|_| Error::Inval)
}

fn group_count_result(size: usize, group_count: usize) -> Result<Option<i64>, Error> {
    let count = i64::try_from(group_count).map_err(|_| Error::Overflow)?;
    if size == 0 {
        Ok(Some(count))
    } else if size < group_count {
        Err(Error::Inval)
    } else {
        Ok(None)
    }
}

fn encode_groups(groups: &[u32]) -> Result<Vec<u8>, Error> {
    let byte_length = groups.len().checked_mul(core::mem::size_of::<u32>())
        .ok_or(Error::Overflow)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(byte_length)
        .map_err(|_| Error::NoMem)?;
    for group in groups {
        output.extend_from_slice(&group.to_ne_bytes());
    }
    Ok(output)
}

impl SetGroups {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::SetGroups;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let count = usize::try_from(frame.arguments[0]).map_err(|_| Error::Inval)?;
        validate_group_count(count)?;
        let process = ThreadManager::current_process()?;
        if !process.effective_user_is_root() {
            return Err(Error::Perm);
        }
        let groups = if count == 0 {
            Vec::new()
        } else {
            let address = usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?;
            if address == 0 {
                return Err(Error::Fault);
            }
            let byte_length = count
                .checked_mul(core::mem::size_of::<u32>())
                .ok_or(Error::Inval)?;
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(byte_length)
                .map_err(|_| Error::NoMem)?;
            bytes.resize(byte_length, 0);
            let address_space = process.address_space();
            address_space
                .read(address, &mut bytes)
                .map_err(|_| Error::Fault)?;
            decode_groups(&bytes)?
        };
        process.set_supplementary_group_ids(&groups)?;
        Ok(0)
    }
}

fn validate_group_count(count: usize) -> Result<(), Error> {
    if count > MAX_SUPPLEMENTARY_GROUPS {
        Err(Error::Inval)
    } else {
        Ok(())
    }
}

fn decode_groups(bytes: &[u8]) -> Result<Vec<u32>, Error> {
    if bytes.len() % core::mem::size_of::<u32>() != 0 {
        return Err(Error::Inval);
    }
    let count = bytes.len() / core::mem::size_of::<u32>();
    let mut groups = Vec::new();
    groups
        .try_reserve_exact(count)
        .map_err(|_| Error::NoMem)?;
    for chunk in bytes.chunks_exact(core::mem::size_of::<u32>()) {
        groups.push(u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(groups)
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/getgroups.rs"]
mod tests;