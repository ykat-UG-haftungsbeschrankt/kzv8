/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::error::Error;

pub(crate) const fn to_syscall_result(error: Error) -> i64 {
    match error {
        Error::Success => 0,
        Error::Min | Error::Max => -22,
        Error::Perm | Error::Error => -1,
        Error::WouldBlock => -11,
        Error::Deadlock => -35,
        Error::Restart => -4,
        _ => error as i64,
    }
}

#[cfg(test)]
#[path = "../../../tests/kernel/syscall/linux/error.rs"]
mod tests;