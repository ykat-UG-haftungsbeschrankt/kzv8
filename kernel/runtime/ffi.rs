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

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "C" fn kz_host_wall_time_ms() -> i64 {
    0
}

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "C" fn kz_host_v8_allocate_pages(
    length: usize,
    alignment: usize,
    permissions: i32,
) -> *mut u8 {
    let Some(address) = crate::physical::allocate_contiguous(length, alignment) else {
        return core::ptr::null_mut();
    };
    if !crate::arch::set_page_permissions(address, length, permissions) {
        let _released = crate::physical::deallocate_contiguous(address, length);
        return core::ptr::null_mut();
    }
    address as *mut u8
}

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "C" fn kz_host_v8_free_pages(address: *mut u8, length: usize) -> i32 {
    if address.is_null() || !crate::arch::set_page_permissions(address as usize, length, 0) {
        return 0;
    }
    crate::physical::deallocate_contiguous(address as usize, length) as i32
}

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "C" fn kz_host_v8_release_pages(
    address: *mut u8,
    length: usize,
    new_length: usize,
) -> i32 {
    if address.is_null() || new_length > length || new_length % 4096 != 0 {
        return 0;
    }
    if new_length == length {
        return 1;
    }
    let Some(released_address) = (address as usize).checked_add(new_length) else {
        return 0;
    };
    kz_host_v8_free_pages(released_address as *mut u8, length - new_length)
}

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "C" fn kz_host_v8_set_page_permissions(
    address: *mut u8,
    length: usize,
    permissions: i32,
) -> i32 {
    if address.is_null() {
        return 0;
    }
    crate::arch::set_page_permissions(address as usize, length, permissions) as i32
}

#[cfg(target_arch = "x86_64")]
pub(crate) fn v8_page_allocator_smoke() -> bool {
    let page_size = 4096;
    let length = page_size * 2;
    let memory = kz_host_v8_allocate_pages(length, page_size, 2);
    if memory.is_null() {
        return false;
    }

    unsafe {
        core::ptr::write_volatile(memory, 0x5a);
        core::ptr::write_volatile(memory.add(page_size), 0xa5);
    }
    let inaccessible = kz_host_v8_set_page_permissions(memory, length, 5) != 0;
    let writable = kz_host_v8_set_page_permissions(memory, length, 2) != 0;
    let contents_match = unsafe {
        core::ptr::read_volatile(memory) == 0x5a
            && core::ptr::read_volatile(memory.add(page_size)) == 0xa5
    };
    let released = kz_host_v8_free_pages(memory, length) != 0;
    inaccessible && writable && contents_match && released
}
