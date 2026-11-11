/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct GetPid;

pub(crate) struct GetPidArguments {}

impl GetPid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::GetPid;

    pub(crate) fn callback(_arguments: GetPidArguments) -> Result<i64, Error> {
        Ok(ThreadManager::current_process()?.id() as i64)
    }

    pub(crate) fn handle(_frame: SyscallFrame) -> Result<i64, Error> {
        Self::callback(GetPidArguments {})
    }
}