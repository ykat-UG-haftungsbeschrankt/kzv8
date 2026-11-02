/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) const INTERRUPT_ID: u32 = 30;

const TIMER_TICK_HZ: u64 = 100;

pub(crate) fn ticks() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {value}, cntvct_el0",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

pub(crate) fn frequency_hz() -> Option<u64> {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {value}, cntfrq_el0",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    (value != 0).then_some(value)
}

pub(crate) fn now_ns() -> Option<u64> {
    let frequency = frequency_hz()?;
    Some((u128::from(ticks()) * 1_000_000_000 / u128::from(frequency)) as u64)
}

pub(crate) fn initialize() {
    arm_timer();
}

pub(crate) fn handle_interrupt() {
    arm_timer();
}

fn arm_timer() {
    let Some(frequency) = frequency_hz() else {
        return;
    };
    let interval = (frequency / TIMER_TICK_HZ).clamp(1, i32::MAX as u64);
    unsafe {
        core::arch::asm!(
            "msr cntp_tval_el0, {interval}",
            "msr cntp_ctl_el0, {enabled}",
            "isb",
            interval = in(reg) interval,
            enabled = in(reg) 1u64,
            options(nostack, preserves_flags),
        );
    }
}
