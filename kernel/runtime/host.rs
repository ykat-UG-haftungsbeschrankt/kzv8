pub(crate) fn console_write(bytes: &[u8]) -> bool {
    crate::console::write(bytes)
}

pub(crate) fn monotonic_time_ns() -> Option<u64> {
    crate::arch::timer::now_ns()
}
