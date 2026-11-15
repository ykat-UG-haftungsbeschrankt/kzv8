/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct YieldExecution;

impl YieldExecution {
    pub(crate) const ID: u64 = 0x0046;

    pub(crate) fn handle(_frame: SyscallFrame) -> Result<u32, crate::kernel::error::Error> {
        if ThreadManager::yield_current() {
            Ok(0)
        } else {
            Ok(super::super::error::STATUS_NO_YIELD_PERFORMED)
        }
    }
}