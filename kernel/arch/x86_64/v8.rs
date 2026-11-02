/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::sync::atomic::{AtomicUsize, Ordering};

static PAGE_ALLOCATOR_TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" {
    fn kz_v8_run_smoke() -> i32;
}

fn debugcon(message: &[u8]) {
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

#[no_mangle]
pub extern "C" fn kz_v8_debug_stage(stage: u8) {
    debugcon(&[stage]);
}

#[no_mangle]
pub extern "C" fn kz_host_v8_allocate_pages(
    address_hint: *mut u8,
    length: usize,
    alignment: usize,
    permissions: i32,
) -> *mut u8 {
    let hint = (!address_hint.is_null()).then_some(address_hint as usize);
    let permissions = if permissions == 5 { 0 } else { permissions };
    let trace = PAGE_ALLOCATOR_TRACE_COUNT.fetch_add(1, Ordering::Relaxed) < 32;
    if trace {
        debugcon(b"A");
    }
    if let Some(address) =
        crate::memory::reserve_with_hint(hint, length, alignment, permissions)
    {
        if trace {
            debugcon(b"H\n");
        }
        return address as *mut u8;
    }
    if hint.is_some() {
        if trace {
            debugcon(b"F");
        }
        if let Some(address) = crate::memory::reserve(length, alignment, permissions) {
            if trace {
                debugcon(b"S\n");
            }
            return address as *mut u8;
        }
    }
    if trace {
        debugcon(b"X\n");
    }
    core::ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn kz_host_v8_random_mmap_addr() -> *mut u8 {
    crate::memory::allocator_base() as *mut u8
}

#[no_mangle]
pub extern "C" fn kz_host_v8_free_pages(address: *mut u8, length: usize) -> i32 {
    if address.is_null() {
        return 0;
    }
    crate::memory::release(address as usize, length) as i32
}

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

#[no_mangle]
pub extern "C" fn kz_host_v8_decommit_pages(address: *mut u8, length: usize) -> i32 {
    if address.is_null() {
        return 0;
    }
    crate::memory::decommit(address as usize, length) as i32
}

#[no_mangle]
pub extern "C" fn kz_host_v8_set_page_permissions(
    address: *mut u8,
    length: usize,
    permissions: i32,
) -> i32 {
    if address.is_null() {
        return 0;
    }
    let permissions = if permissions == 5 { 0 } else { permissions };
    crate::memory::set_permissions(address as usize, length, permissions) as i32
}

pub(crate) fn run_smoke() -> i32 {
    unsafe { kz_v8_run_smoke() }
}

pub(crate) fn page_allocator_smoke() -> bool {
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