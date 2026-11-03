#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
unsafe extern "C" {
    fn kz_v8_run_smoke(
        event_loop: *mut core::ffi::c_void,
        block_device: *mut core::ffi::c_void,
        tcp_context: *mut core::ffi::c_void,
    ) -> i32;
}

#[cfg(target_arch = "x86_64")]
#[no_mangle]
pub extern "C" fn kz_v8_debug_stage(stage: u8) {
    crate::kernel::arch::debug::debugcon(&[stage]);
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[no_mangle]
pub extern "C" fn kz_host_v8_allocate_pages(
    address_hint: *mut u8,
    length: usize,
    alignment: usize,
    permissions: i32,
) -> *mut u8 {
    let hint = (!address_hint.is_null()).then_some(address_hint as usize);
    let permissions = if permissions == 5 { 0 } else { permissions };
    if let Some(address) = crate::kernel::memory::r#virtual::reserve_with_hint(
        hint,
        length,
        alignment,
        permissions,
    ) {
        return address as *mut u8;
    }
    if hint.is_some() {
        if let Some(address) =
            crate::kernel::memory::r#virtual::reserve(length, alignment, permissions)
        {
            return address as *mut u8;
        }
    }
    core::ptr::null_mut()
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[no_mangle]
pub extern "C" fn kz_host_v8_random_mmap_addr() -> *mut u8 {
    crate::kernel::memory::r#virtual::allocator_base() as *mut u8
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[no_mangle]
pub extern "C" fn kz_host_v8_free_pages(address: *mut u8, length: usize) -> i32 {
    if address.is_null() {
        return 0;
    }
    crate::kernel::memory::r#virtual::release(address as usize, length) as i32
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[no_mangle]
pub extern "C" fn kz_host_v8_release_pages(
    address: *mut u8,
    length: usize,
    new_length: usize,
) -> i32 {
    if address.is_null() || new_length > length || new_length % 4096 != 0 {
        return 0;
    }
    crate::kernel::memory::r#virtual::release_suffix(address as usize, length, new_length)
        as i32
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
#[no_mangle]
pub extern "C" fn kz_host_v8_decommit_pages(address: *mut u8, length: usize) -> i32 {
    if address.is_null() {
        return 0;
    }
    crate::kernel::memory::r#virtual::decommit(address as usize, length) as i32
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
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
    crate::kernel::memory::r#virtual::set_permissions(address as usize, length, permissions)
        as i32
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(crate) fn run_smoke(
    event_loop: &mut crate::kernel::event::r#loop::EventLoop<16>,
    block_device: *mut core::ffi::c_void,
    tcp_context: *mut core::ffi::c_void,
) -> Option<i32> {
    Some(unsafe {
        kz_v8_run_smoke(
            event_loop as *mut _ as *mut core::ffi::c_void,
            block_device,
            tcp_context,
        )
    })
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub(crate) fn run_smoke(
    _event_loop: &mut crate::kernel::event::r#loop::EventLoop<16>,
    _block_device: *mut core::ffi::c_void,
    _tcp_context: *mut core::ffi::c_void,
) -> Option<i32> {
    None
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(crate) fn page_allocator_smoke() -> bool {
    let page_size = 4096;
    let length = page_size * 2;
    let out_of_arena_hint = crate::kernel::memory::r#virtual::allocator_base()
        .saturating_sub(page_size) as *mut u8;
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

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub(crate) fn page_allocator_smoke() -> bool {
    false
}
