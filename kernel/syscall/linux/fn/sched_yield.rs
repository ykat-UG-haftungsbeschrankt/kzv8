/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct SchedYield;

pub(crate) struct SchedYieldArguments {}

impl SchedYield {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::SchedYield;

    pub(crate) fn callback(_arguments: SchedYieldArguments) -> Result<i64, Error> {
        let _switched = ThreadManager::yield_current();
        Ok(0)
    }

    pub(crate) fn handle(_frame: SyscallFrame) -> Result<i64, Error> {
        Self::callback(SchedYieldArguments {})
    }
}