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
    if crate::runtime_host::console_write(bytes) {
        length as i32
    } else {
        -1
    }
}

#[no_mangle]
pub extern "C" fn kz_host_time_ns() -> u64 {
    crate::runtime_host::monotonic_time_ns().unwrap_or(0)
}
