unsafe extern "C" {
    fn kz_v8_run_smoke(
        event_loop: *mut core::ffi::c_void,
        block_device: *mut core::ffi::c_void,
        tcp_context: *mut core::ffi::c_void,
        bootstrap_source: *const u8,
        bootstrap_length: usize,
    ) -> i32;
}

#[no_mangle]
#[cfg(target_arch = "x86_64")]
pub extern "C" fn kz_v8_debug_stage(stage: u8) {
    crate::kernel::arch::debug::debugcon(&[stage]);
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

#[no_mangle]
pub extern "C" fn kz_host_v8_random_mmap_addr() -> *mut u8 {
    crate::kernel::memory::r#virtual::allocator_base() as *mut u8
}

#[no_mangle]
pub extern "C" fn kz_host_v8_free_pages(address: *mut u8, length: usize) -> i32 {
    if address.is_null() {
        return 0;
    }
    crate::kernel::memory::r#virtual::release(address as usize, length) as i32
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
    crate::kernel::memory::r#virtual::release_suffix(address as usize, length, new_length)
        as i32
}

#[no_mangle]
pub extern "C" fn kz_host_v8_decommit_pages(address: *mut u8, length: usize) -> i32 {
    if address.is_null() {
        return 0;
    }
    crate::kernel::memory::r#virtual::decommit(address as usize, length) as i32
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
    crate::kernel::memory::r#virtual::set_permissions(address as usize, length, permissions)
        as i32
}

pub(crate) fn run_smoke(
    event_loop: &mut crate::kernel::event::r#loop::EventLoop<16>,
    block_device: *mut core::ffi::c_void,
    tcp_context: *mut core::ffi::c_void,
) -> Option<i32> {
    let bootstrap_source = include_bytes!("../app/dist/vfat.js");
    Some(unsafe {
        kz_v8_run_smoke(
            event_loop as *mut _ as *mut core::ffi::c_void,
            block_device,
            tcp_context,
            bootstrap_source.as_ptr(),
            bootstrap_source.len(),
        )
    })
}