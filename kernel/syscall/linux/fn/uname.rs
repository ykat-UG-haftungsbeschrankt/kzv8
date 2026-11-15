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

const FIELD_LENGTH: usize = 65;
const UNAME_LENGTH: usize = FIELD_LENGTH * 6;

pub(crate) struct Uname;

impl Uname {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Uname;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let address = usize::try_from(frame.arguments[0]).map_err(|_| Error::Fault)?;
        if address == 0 {
            return Err(Error::Fault);
        }
        let output = utsname_bytes();
        let process = ThreadManager::current_process()?;
        let mut address_space = process.address_space();
        address_space
            .write(address, &output)
            .map_err(|_| Error::Fault)?;
        Ok(0)
    }
}

fn utsname_bytes() -> [u8; UNAME_LENGTH] {
    let mut output = [0; UNAME_LENGTH];
    write_field(&mut output, 0, b"Linux");
    write_field(&mut output, 1, b"localhost");
    write_field(&mut output, 2, b"6.0.0-kz");
    write_field(&mut output, 3, b"#1 KZ Kernel");
    #[cfg(target_arch = "x86_64")]
    write_field(&mut output, 4, b"x86_64");
    #[cfg(target_arch = "aarch64")]
    write_field(&mut output, 4, b"aarch64");
    write_field(&mut output, 5, b"localdomain");
    output
}

fn write_field(output: &mut [u8; UNAME_LENGTH], field: usize, value: &[u8]) {
    let start = field * FIELD_LENGTH;
    let length = value.len().min(FIELD_LENGTH - 1);
    output[start..start + length].copy_from_slice(&value[..length]);
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/uname.rs"]
mod tests;