/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::error::Error;

pub(crate) fn handle() -> Result<i64, Error> {
    Err(Error::NoSys)
}

pub(crate) fn handle_syscall(_number: u64) -> Result<i64, Error> {
    if _number == 435 {
        crate::kernel::debug::debug("kz: unsupported clone3 syscall\n");
    }
    handle()
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/unsupported.rs"]
mod tests;