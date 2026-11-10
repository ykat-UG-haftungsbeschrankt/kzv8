/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct ExitGroup;

pub(crate) struct ExitGroupArguments {}

impl ExitGroup {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::ExitGroup;

    pub(crate) fn callback(_arguments: ExitGroupArguments) -> Result<i64, Error> {
        ThreadManager::exit_current(core::ptr::null_mut())
    }

    pub(crate) fn handle(_frame: SyscallFrame) -> Result<i64, Error> {
        Self::callback(ExitGroupArguments {})
    }
}