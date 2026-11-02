/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::sync::atomic::{AtomicUsize, Ordering};

#[cfg(target_arch = "x86_64")]
static V8_PAGE_ALLOCATOR_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);

#[cfg(target_arch = "x86_64")]
fn v8_debugcon(message: &[u8]) {
    for byte in message {
        unsafe {
            core::arch::asm!(
                "out dx, al",
                in("dx") 0xe9u16,
                in("al") *byte,
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "C" fn kz_v8_debug_stage(stage: u8) {
    v8_debugcon(&[stage]);
}

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
    match crate::runtime_host::monotonic_time_ns() {
        Some(now_ns) if now_ns != 0 => now_ns,
        #[cfg(target_arch = "x86_64")]
        _ => crate::arch::timer::ticks(),
        #[cfg(target_arch = "aarch64")]
        _ => 0,
    }
}

#[no_mangle]
pub extern "C" fn kz_host_wall_time_ms() -> i64 {
    0
}

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "C" fn kz_host_v8_allocate_pages(
    address_hint: *mut u8,
    length: usize,
    alignment: usize,
    permissions: i32,
) -> *mut u8 {
    let hint = (!address_hint.is_null()).then_some(address_hint as usize);
    let permissions = if permissions == 5 { 0 } else { permissions };
    let trace = V8_PAGE_ALLOCATOR_TRACE_COUNT.fetch_add(1, Ordering::Relaxed) < 32;
    if trace {
        v8_debugcon(b"A");
    }
    if let Some(address) =
        crate::memory::reserve_with_hint(hint, length, alignment, permissions)
    {
        if trace {
            v8_debugcon(b"H\n");
        }
        return address as *mut u8;
    }
    if hint.is_some() {
        if trace {
            v8_debugcon(b"F");
        }
        if let Some(address) = crate::memory::reserve(length, alignment, permissions) {
            if trace {
                v8_debugcon(b"S\n");
            }
            return address as *mut u8;
        }
    }
    if trace {
        v8_debugcon(b"X\n");
    }
    core::ptr::null_mut()
}

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "C" fn kz_host_v8_random_mmap_addr() -> *mut u8 {
    crate::memory::allocator_base() as *mut u8
}

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "C" fn kz_host_v8_free_pages(address: *mut u8, length: usize) -> i32 {
    if address.is_null() {
        return 0;
    }
    crate::memory::release(address as usize, length) as i32
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
    crate::memory::release_suffix(address as usize, length, new_length) as i32
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
    crate::memory::set_permissions(address as usize, length, permissions) as i32
}

#[cfg(target_arch = "x86_64")]
pub(crate) fn v8_page_allocator_smoke() -> bool {
    let page_size = 4096;
    let length = page_size * 2;
    let out_of_arena_hint =
        crate::memory::allocator_base().saturating_sub(page_size) as *mut u8;
    let memory = kz_host_v8_allocate_pages(out_of_arena_hint, length, page_size, 2);
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
