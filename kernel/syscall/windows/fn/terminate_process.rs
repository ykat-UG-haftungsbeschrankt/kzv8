/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::c_void;

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) struct TerminateProcess;

impl TerminateProcess {
    pub(crate) const ID: u64 = 0x002c;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<u32, Error> {
        if frame.windows_arguments[0] != u64::MAX {
            return Err(Error::Badf);
        }
        let exit_status = frame.windows_arguments[1] as u32 as usize as *mut c_void;
        ThreadManager::exit_current(exit_status)
    }
}