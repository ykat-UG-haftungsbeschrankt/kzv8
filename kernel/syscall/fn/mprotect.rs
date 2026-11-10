/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{protection_to_vm, PAGE_SIZE};
use super::super::SyscallFrame;
use crate::kernel::error::Error;

pub(crate) struct Mprotect;

pub(crate) struct MprotectArguments {
    pub(crate) address: u64,
    pub(crate) length: u64,
    pub(crate) protection: u64,
}

impl Mprotect {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Mprotect;

    pub(crate) fn callback(arguments: MprotectArguments) -> Result<i64, Error> {
        let Ok(address) = usize::try_from(arguments.address) else {
            return Err(Error::Inval);
        };
        let Ok(length) = usize::try_from(arguments.length) else {
            return Err(Error::Inval);
        };
        if address % PAGE_SIZE != 0 || length == 0 {
            return Err(Error::Inval);
        }
        let Some(permissions) = protection_to_vm(arguments.protection) else {
            return Err(Error::Inval);
        };
        if crate::kernel::memory::r#virtual::protect_user_mapping(address, length, permissions) {
            Ok(0)
        } else {
            Err(Error::NoMem)
        }
    }

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        Self::callback(MprotectArguments {
            address: frame.arguments[0],
            length: frame.arguments[1],
            protection: frame.arguments[2],
        })
    }
}