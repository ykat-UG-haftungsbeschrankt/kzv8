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

const ARCH_SET_FS: u64 = 0x1002;
const ARCH_GET_FS: u64 = 0x1003;

pub(crate) struct ArchPrctl;

impl ArchPrctl {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::ArchPrctl;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let process = ThreadManager::current_process()?;
        let address = usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?;
        match frame.arguments[0] {
            ARCH_SET_FS => {
                if address != 0 && !process.address_space().user_range_accessible(address, 1, false) {
                    return Err(Error::Fault);
                }
                crate::kernel::arch::set_user_fs_base(address as u64);
                Ok(0)
            }
            ARCH_GET_FS => {
                let mut address_space = process.address_space();
                address_space
                    .write(address, &crate::kernel::arch::user_fs_base().to_ne_bytes())
                    .map_err(|_| Error::Fault)?;
                Ok(0)
            }
            _ => Err(Error::NoSys),
        }
    }
}