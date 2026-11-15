/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct GetTid;

pub(crate) struct GetTidArguments {}

impl GetTid {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::GetTid;

    pub(crate) fn callback(_arguments: GetTidArguments) -> Result<i64, Error> {
        Ok(ThreadManager::current_thread_id() as i64)
    }

    pub(crate) fn handle(_frame: SyscallFrame) -> Result<i64, Error> {
        Self::callback(GetTidArguments {})
    }
}