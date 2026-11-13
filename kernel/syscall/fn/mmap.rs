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
const MAP_PRIVATE: u32 = 2;
const MAP_ANONYMOUS: u32 = 0x20;
const MAP_NORESERVE: u32 = 0x4000;

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
        let flags = arguments.flags as u32;
        if length == 0 {
            return Err(Error::Inval);
        }
        if length > MAX_MMAP_LENGTH {
            return Err(Error::NoMem);
        }
        if arguments.protection & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0
            || flags & !(MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE) != 0
            || flags & (MAP_PRIVATE | MAP_ANONYMOUS) != (MAP_PRIVATE | MAP_ANONYMOUS)
            || arguments.file_descriptor != u64::MAX
            || arguments.offset != 0
            || arguments.protection & (PROT_WRITE | PROT_EXEC) == (PROT_WRITE | PROT_EXEC)
        {
            return Err(Error::Inval);
        }
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