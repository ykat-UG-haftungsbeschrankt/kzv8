/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) fn console_write(bytes: &[u8]) -> bool {
    crate::console::write(bytes)
}

pub(crate) fn monotonic_time_ns() -> Option<u64> {
    crate::arch::timer::now_ns()
}
