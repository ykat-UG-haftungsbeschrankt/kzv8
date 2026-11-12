/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::arch::traits::CpuArchitecture as _;
use crate::kernel::error::Error;

pub(crate) fn fill(output: &mut [u8]) -> Result<(), Error> {
    for chunk in output.chunks_mut(core::mem::size_of::<u32>()) {
        let random = crate::kernel::arch::CpuArchitecture::random_u32().ok_or(Error::NoSys)?;
        let bytes = random.to_ne_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
    Ok(())
}
