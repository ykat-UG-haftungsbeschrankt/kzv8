/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[no_mangle]
pub unsafe extern "C" fn kz_host_console_write(data: *const u8, length: usize) -> i32 {
    if length > 512 || (data.is_null() && length != 0) {
        return -1;
    }
    let bytes = if length == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(data, length)
    };
    if crate::console::write(bytes) {
        length as i32
    } else {
        -1
    }
}

#[no_mangle]
pub extern "C" fn kz_host_time_ns() -> u64 {
    match crate::arch::timer::now_ns() {
        Some(now_ns) if now_ns != 0 => now_ns,
        _ => crate::arch::fallback_time_ns(),
    }
}

#[no_mangle]
pub extern "C" fn kz_host_wall_time_ms() -> i64 {
    0
}