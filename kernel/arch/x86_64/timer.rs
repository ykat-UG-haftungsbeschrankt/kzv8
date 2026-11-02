/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::arch::x86_64::__cpuid;

pub(crate) fn initialize() {}

pub(crate) fn ticks() -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    (u64::from(high) << 32) | u64::from(low)
}

pub(crate) fn frequency_hz() -> Option<u64> {
    let maximum = __cpuid(0).eax;
    if maximum >= 0x15 {
        let ratio = __cpuid(0x15);
        if ratio.eax != 0 && ratio.ebx != 0 && ratio.ecx != 0 {
            return Some(u64::from(ratio.ecx) * u64::from(ratio.ebx) / u64::from(ratio.eax));
        }
    }
    if maximum >= 0x16 {
        let nominal = __cpuid(0x16).eax;
        if nominal != 0 {
            return Some(u64::from(nominal) * 1_000_000);
        }
    }
    None
}

pub(crate) fn now_ns() -> Option<u64> {
    let frequency = frequency_hz()?;
    Some((u128::from(ticks()) * 1_000_000_000 / u128::from(frequency)) as u64)
}
