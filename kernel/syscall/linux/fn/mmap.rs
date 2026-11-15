/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{protection_to_vm, PAGE_SIZE, PROT_EXEC, PROT_READ, PROT_WRITE};
use super::super::SyscallFrame;
use crate::kernel::dl::AddressSpace as _;
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::scheduler::thread::{vm::UserAddressSpace, ThreadManager};
use crate::kernel::vfs::{FileHandleApi as _, SeekFrom};

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
const MAP_IGNORED_FLAGS: u64 = MAP_DENYWRITE | MAP_EXECUTABLE;
const MAP_FIXED_FLAGS: u64 = MAP_FIXED | MAP_FIXED_NOREPLACE;
const MAP_UNSUPPORTED_FLAGS: u64 = MAP_32BIT
    | MAP_GROWSDOWN
    | MAP_LOCKED
    | MAP_POPULATE
    | MAP_NONBLOCK
    | MAP_STACK
    | MAP_HUGETLB
    | MAP_SYNC
    | MAP_UNINITIALIZED;
const MAP_KNOWN_FLAGS: u64 = MAP_TYPE_MASK
    | MAP_ANONYMOUS
    | MAP_NORESERVE
    | MAP_IGNORED_FLAGS
    | MAP_FIXED_FLAGS
    | MAP_UNSUPPORTED_FLAGS;

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
        let length = usize::try_from(arguments.length).map_err(|_| Error::NoMem)?;
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
        let process = ThreadManager::current_process()?;
        let (address_space_limit, _) = process.resource_limit(9)?;
        let Some(rounded_length) = length
            .checked_add(PAGE_SIZE - 1)
            .map(|length| length & !(PAGE_SIZE - 1))
        else {
            return Err(Error::NoMem);
        };
        let mapped_bytes = u64::try_from(rounded_length).map_err(|_| Error::NoMem)?;
        let mut address_space = process.address_space();
        let address_hint = usize::try_from(arguments.address_hint).map_err(|_| Error::Inval)?;
        let fixed = arguments.flags & MAP_FIXED_FLAGS != 0;
        let hint = if fixed {
            if address_hint % PAGE_SIZE != 0
                || arguments.flags & MAP_FIXED != 0
                    && arguments.flags & MAP_FIXED_NOREPLACE != 0
            {
                return Err(Error::Inval);
            }
            Some(address_hint)
        } else if address_hint == 0 {
            None
        } else {
            Some(address_hint & !(PAGE_SIZE - 1))
        };

        let already_reserved = fixed
            && address_space.user_range_reserved(hint.ok_or(Error::Inval)?, rounded_length);
        if arguments.flags & MAP_FIXED_NOREPLACE != 0 && already_reserved {
            return Err(Error::Exist);
        }
        if !already_reserved {
            process.try_reserve_memory(mapped_bytes)?;
        }

        let result = if arguments.flags & MAP_ANONYMOUS != 0 {
            map_anonymous(
                &mut address_space,
                hint,
                length,
                rounded_length,
                permissions,
                address_space_limit,
                fixed,
                already_reserved,
            )
        } else {
            map_file(
                &process,
                &mut address_space,
                hint,
                length,
                rounded_length,
                permissions,
                arguments.file_descriptor,
                arguments.offset,
                address_space_limit,
                fixed,
                already_reserved,
            )
        };
        if result.is_err() && !already_reserved {
            process.release_memory(mapped_bytes, mapped_bytes, 0);
        }
        result.map(|address| address as i64)
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

fn map_anonymous(
    address_space: &mut UserAddressSpace,
    hint: Option<usize>,
    length: usize,
    rounded_length: usize,
    permissions: i32,
    address_space_limit: u64,
    fixed: bool,
    already_reserved: bool,
) -> Result<usize, Error> {
    if already_reserved {
        let address = hint.ok_or(Error::Inval)?;
        if !address_space.protect_user_mapping(address, rounded_length, 2) {
            return Err(Error::NoMem);
        }
        address_space.zero(address, rounded_length)?;
        if !address_space.protect_user_mapping(address, rounded_length, permissions) {
            return Err(Error::NoMem);
        }
        return Ok(address);
    }
    let address = address_space
        .reserve_user_mapping_limited(hint, length, permissions, address_space_limit)
        .ok_or(Error::NoMem)?;
    if fixed && Some(address) != hint {
        let _released = address_space.release_user_mapping(address, rounded_length);
        return Err(Error::NoMem);
    }
    Ok(address)
}

fn map_file(
    process: &Process,
    address_space: &mut UserAddressSpace,
    hint: Option<usize>,
    length: usize,
    rounded_length: usize,
    permissions: i32,
    file_descriptor: u64,
    offset: u64,
    address_space_limit: u64,
    fixed: bool,
    already_reserved: bool,
) -> Result<usize, Error> {
    let offset = usize::try_from(offset).map_err(|_| Error::Inval)?;
    let descriptor = u32::try_from(file_descriptor).map_err(|_| Error::Badf)?;
    let object = process.file_descriptor(descriptor)?;
    let mut file = object.take_file()?;
    let address = if already_reserved {
        hint.ok_or(Error::Inval)?
    } else {
        let result = address_space
            .reserve_user_mapping_limited(hint, length, 2, address_space_limit)
            .ok_or(Error::NoMem);
        match result {
            Ok(address) if !fixed || Some(address) == hint => address,
            Ok(address) => {
                let _released = address_space.release_user_mapping(address, rounded_length);
                object.restore_file(file)?;
                return Err(Error::NoMem);
            }
            Err(error) => {
                object.restore_file(file)?;
                return Err(error);
            }
        }
    };

    let mapped = (|| {
        if !address_space.protect_user_mapping(address, rounded_length, 2) {
            return Err(Error::NoMem);
        }
        address_space.zero(address, rounded_length)?;
        file.seek(SeekFrom::Start(u64::try_from(offset).map_err(|_| Error::Inval)?))?;
        let mut copied = 0usize;
        let mut buffer = [0; PAGE_SIZE];
        while copied < length {
            let chunk_length = (length - copied).min(buffer.len());
            let count = file.read_blocking(&mut buffer[..chunk_length])?;
            if count == 0 {
                break;
            }
            if count > chunk_length {
                return Err(Error::Io);
            }
            address_space.write(
                address.checked_add(copied).ok_or(Error::Overflow)?,
                &buffer[..count],
            )?;
            copied = copied.checked_add(count).ok_or(Error::Overflow)?;
        }
        if !address_space.protect_user_mapping(address, rounded_length, permissions) {
            return Err(Error::NoMem);
        }
        Ok(address)
    })();
    object.restore_file(file)?;

    if mapped.is_err() && !already_reserved {
        let _released = address_space.release_user_mapping(address, rounded_length);
    }
    mapped
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
    if offset % PAGE_SIZE as u64 != 0 {
        return Err(Error::Inval);
    }
    if flags & MAP_ANONYMOUS != 0 && offset != 0 {
        return Err(Error::Inval);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/mmap.rs"]
mod tests;