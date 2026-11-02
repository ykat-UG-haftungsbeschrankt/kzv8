/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::arch::x86_64::__cpuid;

const TIMER_TICK_HZ: u64 = 100;
const CALIBRATION_WINDOW_MS: u64 = 20;
const FALLBACK_INITIAL_COUNT: u32 = 100_000;

pub(crate) fn initialize() {
    let initial_count = frequency_hz()
        .and_then(|frequency| {
            let calibration_cycles = frequency.saturating_mul(CALIBRATION_WINDOW_MS) / 1000;
            if calibration_cycles == 0 {
                return None;
            }

            crate::kernel::arch::interrupts::begin_timer_calibration();
            let start = ticks();
            while ticks().wrapping_sub(start) < calibration_cycles {
                core::hint::spin_loop();
            }

            let elapsed_count = u64::from(
                u32::MAX.wrapping_sub(crate::kernel::arch::interrupts::local_timer_current_count()),
            );
            let initial_count =
                elapsed_count.saturating_mul(1000 / TIMER_TICK_HZ) / CALIBRATION_WINDOW_MS;
            (initial_count > 0 && initial_count <= u64::from(u32::MAX))
                .then_some(initial_count as u32)
        })
        .unwrap_or(FALLBACK_INITIAL_COUNT);
    crate::kernel::arch::interrupts::initialize_timer(initial_count);
}

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
