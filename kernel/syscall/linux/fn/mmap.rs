/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{protection_to_vm, PAGE_SIZE, PROT_EXEC, PROT_READ, PROT_WRITE};
use super::super::SyscallFrame;
use crate::kernel::error::Error;

const MAX_MMAP_LENGTH: usize = 1 << 30;
const MAP_TYPE_MASK: u64 = 0x0f;
const MAP_SHARED: u64 = 0x01;
const MAP_PRIVATE: u64 = 0x02;
const MAP_SHARED_VALIDATE: u64 = 0x03;
const MAP_FIXED: u64 = 0x10;
const MAP_32BIT: u64 = 0x40;
const MAP_ANONYMOUS: u64 = 0x20;
const MAP_GROWSDOWN: u64 = 0x100;
const MAP_DENYWRITE: u64 = 0x800;
const MAP_EXECUTABLE: u64 = 0x1000;
const MAP_LOCKED: u64 = 0x2000;
const MAP_NORESERVE: u64 = 0x4000;
const MAP_POPULATE: u64 = 0x8000;
const MAP_NONBLOCK: u64 = 0x10000;
const MAP_STACK: u64 = 0x20000;
const MAP_HUGETLB: u64 = 0x40000;
const MAP_SYNC: u64 = 0x80000;
const MAP_FIXED_NOREPLACE: u64 = 0x100000;
const MAP_UNINITIALIZED: u64 = 0x4000000;
const MAP_SUPPORTED_FLAGS: u64 = MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE;
const MAP_UNSUPPORTED_FLAGS: u64 = MAP_FIXED
    | MAP_32BIT
    | MAP_GROWSDOWN
    | MAP_DENYWRITE
    | MAP_EXECUTABLE
    | MAP_LOCKED
    | MAP_POPULATE
    | MAP_NONBLOCK
    | MAP_STACK
    | MAP_HUGETLB
    | MAP_SYNC
    | MAP_FIXED_NOREPLACE
    | MAP_UNINITIALIZED;
const MAP_KNOWN_FLAGS: u64 = MAP_TYPE_MASK | MAP_ANONYMOUS | MAP_NORESERVE | MAP_UNSUPPORTED_FLAGS;

pub(crate) struct Mmap;

pub(crate) struct MmapArguments {
    pub(crate) address_hint: u64,
    pub(crate) length: u64,
    pub(crate) protection: u64,
    pub(crate) flags: u64,
    pub(crate) file_descriptor: u64,
    pub(crate) offset: u64,
}

impl Mmap {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Mmap;

    pub(crate) fn callback(arguments: MmapArguments) -> Result<i64, Error> {
        let Ok(length) = usize::try_from(arguments.length) else {
            return Err(Error::NoMem);
        };
        if length == 0 {
            return Err(Error::Inval);
        }
        if length > MAX_MMAP_LENGTH {
            return Err(Error::NoMem);
        }
        if arguments.protection & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0
            || arguments.protection & (PROT_WRITE | PROT_EXEC) == (PROT_WRITE | PROT_EXEC)
        {
            return Err(Error::Inval);
        }
        validate_mapping_mode(arguments.flags, arguments.offset)?;
        let Some(permissions) = protection_to_vm(arguments.protection) else {
            return Err(Error::Inval);
        };
        let process = crate::kernel::scheduler::thread::ThreadManager::current_process()?;
        let (address_space_limit, _) = process.resource_limit(9)?;
        let Some(rounded_length) = length
            .checked_add(PAGE_SIZE - 1)
            .map(|length| length & !(PAGE_SIZE - 1))
        else {
            return Err(Error::NoMem);
        };
        let mapped_bytes = u64::try_from(rounded_length).map_err(|_| Error::NoMem)?;
        process.try_reserve_memory(mapped_bytes)?;
        let address_space = process.address_space();
        let hint = (arguments.address_hint != 0).then_some(arguments.address_hint as usize);
        match address_space.reserve_user_mapping_limited(
            hint,
            length,
            permissions,
            address_space_limit,
        ) {
            Some(address) => Ok(address as i64),
            None => {
                process.release_memory(mapped_bytes, mapped_bytes, 0);
                Err(Error::NoMem)
            }
        }
    }

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        Self::callback(MmapArguments {
            address_hint: frame.arguments[0],
            length: frame.arguments[1],
            protection: frame.arguments[2],
            flags: frame.arguments[3],
            file_descriptor: frame.arguments[4],
            offset: frame.arguments[5],
        })
    }
}

fn validate_mapping_mode(flags: u64, offset: u64) -> Result<(), Error> {
    if flags & !MAP_KNOWN_FLAGS != 0 {
        return Err(Error::Inval);
    }
    if flags & MAP_UNSUPPORTED_FLAGS != 0 {
        return Err(Error::NoSys);
    }
    if !matches!(flags & MAP_TYPE_MASK, MAP_PRIVATE) {
        return if matches!(flags & MAP_TYPE_MASK, MAP_SHARED | MAP_SHARED_VALIDATE) {
            Err(Error::NoSys)
        } else {
            Err(Error::Inval)
        };
    }
    if flags & MAP_ANONYMOUS == 0 {
        return Err(Error::NoSys);
    }
    if offset % PAGE_SIZE as u64 != 0 {
        return Err(Error::Inval);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/mmap.rs"]
mod tests;