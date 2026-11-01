pub(crate) fn initialize() {
    let cr0: u64;
    unsafe {
        core::arch::asm!(
            "mov {value}, cr0",
            value = out(reg) cr0,
            options(nomem, nostack, preserves_flags),
        );
    }
    assert!(cr0 & (1 << 31) != 0, "x86 paging was not enabled by boot");
}
