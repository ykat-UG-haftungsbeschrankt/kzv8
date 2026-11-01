pub(crate) fn initialize() {}

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

pub(crate) fn frequency_hz() -> u64 {
    let value: u64;
    unsafe {
        core::arch::asm!(
            "mrs {value}, cntfrq_el0",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

pub(crate) fn now_ns() -> Option<u64> {
    let frequency = frequency_hz();
    (frequency != 0).then(|| (u128::from(ticks()) * 1_000_000_000 / u128::from(frequency)) as u64)
}
