/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct GetUid;
pub(crate) struct GetEuid;
pub(crate) struct GetGid;
pub(crate) struct GetEgid;
pub(crate) struct GetResUid;
pub(crate) struct GetResGid;
pub(crate) struct SetUid;
pub(crate) struct SetGid;
pub(crate) struct SetReUid;
pub(crate) struct SetReGid;
pub(crate) struct SetResUid;
pub(crate) struct SetResGid;

fn optional_id(value: u64) -> Option<u32> {
    let identifier = value as u32;
    (identifier != u32::MAX).then_some(identifier)
}

fn write_ids(frame: SyscallFrame, user: bool) -> Result<i64, Error> {
    let process = ThreadManager::current_process()?;
    let identifiers = if user {
        process.user_ids()
    } else {
        process.group_ids()
    };
    let mut address_space = process.address_space();
    for (address, identifier) in frame.arguments[..3].iter().copied().zip(identifiers) {
        let address = usize::try_from(address).map_err(|_| Error::Fault)?;
        if address == 0 {
            return Err(Error::Fault);
        }
        address_space
            .write(address, &identifier.to_ne_bytes())
            .map_err(|_| Error::Fault)?;
    }
    Ok(0)
}

fn current_id(user: bool, index: usize) -> Result<i64, Error> {
    let process = ThreadManager::current_process()?;
    let identifiers = if user {
        process.user_ids()
    } else {
        process.group_ids()
    };
    Ok(i64::from(identifiers[index]))
}

impl GetUid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::GetUid;

    pub(crate) fn handle(_frame: SyscallFrame) -> Result<i64, Error> {
        current_id(true, 0)
    }
}

impl GetEuid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::GetEuid;

    pub(crate) fn handle(_frame: SyscallFrame) -> Result<i64, Error> {
        current_id(true, 1)
    }
}

impl GetGid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::GetGid;

    pub(crate) fn handle(_frame: SyscallFrame) -> Result<i64, Error> {
        current_id(false, 0)
    }
}

impl GetEgid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::GetEgid;

    pub(crate) fn handle(_frame: SyscallFrame) -> Result<i64, Error> {
        current_id(false, 1)
    }
}

impl GetResUid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Getresuid;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        write_ids(frame, true)
    }
}

impl GetResGid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Getresgid;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        write_ids(frame, false)
    }
}

impl SetUid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Setuid;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        ThreadManager::current_process()?.set_user_id(frame.arguments[0] as u32)?;
        Ok(0)
    }
}

impl SetGid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Setgid;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        ThreadManager::current_process()?.set_group_id(frame.arguments[0] as u32)?;
        Ok(0)
    }
}

impl SetReUid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Setreuid;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        ThreadManager::current_process()?.set_real_and_effective_user(
            optional_id(frame.arguments[0]),
            optional_id(frame.arguments[1]),
        )?;
        Ok(0)
    }
}

impl SetReGid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Setregid;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        ThreadManager::current_process()?.set_real_and_effective_group(
            optional_id(frame.arguments[0]),
            optional_id(frame.arguments[1]),
        )?;
        Ok(0)
    }
}

impl SetResUid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Setresuid;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        ThreadManager::current_process()?.set_all_user_ids(
            optional_id(frame.arguments[0]),
            optional_id(frame.arguments[1]),
            optional_id(frame.arguments[2]),
        )?;
        Ok(0)
    }
}

impl SetResGid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Setresgid;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        ThreadManager::current_process()?.set_all_group_ids(
            optional_id(frame.arguments[0]),
            optional_id(frame.arguments[1]),
            optional_id(frame.arguments[2]),
        )?;
        Ok(0)
    }
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/credentials.rs"]
mod tests;