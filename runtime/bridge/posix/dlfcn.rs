use core::ffi::{c_char, c_int, c_void};
use core::sync::atomic::{AtomicBool, Ordering};
use super::errno::ENOSYS;
static ERROR_PENDING: AtomicBool = AtomicBool::new(false);
static ERROR_MESSAGE: &[u8] = b"dynamic loading is not supported\0";

#[repr(C)]
pub struct DlInfo {
    file_name: *const c_char,
    file_base: *mut c_void,
    symbol_name: *const c_char,
    symbol_address: *mut c_void,
}

// FIXME: The kernel has no ELF shared-object loader, relocation engine, or dynamic symbol table.
fn set_loader_error() {
    ERROR_PENDING.store(true, Ordering::Release);
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
}

#[no_mangle]
pub unsafe extern "C" fn dladdr(_address: *const c_void, information: *mut DlInfo) -> c_int {
    if !information.is_null() {
        unsafe {
            information.write(DlInfo {
                file_name: core::ptr::null(),
                file_base: core::ptr::null_mut(),
                symbol_name: core::ptr::null(),
                symbol_address: core::ptr::null_mut(),
            });
        }
    }
    set_loader_error();
    0
}

#[no_mangle]
pub extern "C" fn dlclose(_handle: *mut c_void) -> c_int {
    set_loader_error();
    -1
}

#[no_mangle]
pub extern "C" fn dlerror() -> *mut c_char {
    if ERROR_PENDING.swap(false, Ordering::AcqRel) {
        ERROR_MESSAGE.as_ptr().cast_mut().cast()
    } else {
        core::ptr::null_mut()
    }
}

#[no_mangle]
pub extern "C" fn dlopen(_file: *const c_char, _mode: c_int) -> *mut c_void {
    set_loader_error();
    core::ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn dlsym(_handle: *mut c_void, _symbol: *const c_char) -> *mut c_void {
    set_loader_error();
    core::ptr::null_mut()
}