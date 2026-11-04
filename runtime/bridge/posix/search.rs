use core::ffi::{c_char, c_int, c_void};
use core::ptr::{copy_nonoverlapping, read_volatile, write_volatile};
use super::errno::ENOSYS;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Entry {
    key: *mut c_char,
    data: *mut c_void,
}

#[no_mangle]
pub unsafe extern "C" fn lfind(
    key: *const c_void,
    base: *const c_void,
    element_count: *mut usize,
    element_size: usize,
    compare: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> c_int>,
) -> *mut c_void {
    if key.is_null() || base.is_null() || element_count.is_null() || element_size == 0 {
        return core::ptr::null_mut();
    }
    let Some(compare) = compare else {
        return core::ptr::null_mut();
    };
    let count = unsafe { read_volatile(element_count) };
    for index in 0..count {
        let candidate = unsafe { base.cast::<u8>().add(index * element_size) };
        if unsafe { compare(key, candidate.cast()) } == 0 {
            return candidate.cast_mut().cast();
        }
    }
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn lsearch(
    key: *const c_void,
    base: *mut c_void,
    element_count: *mut usize,
    element_size: usize,
    compare: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> c_int>,
) -> *mut c_void {
    let found = unsafe { lfind(key, base, element_count, element_size, compare) };
    if !found.is_null() {
        return found;
    }
    if key.is_null() || base.is_null() || element_count.is_null() || element_size == 0 {
        return core::ptr::null_mut();
    }
    let count = unsafe { read_volatile(element_count) };
    let Some(offset) = count.checked_mul(element_size) else {
        return core::ptr::null_mut();
    };
    let slot = unsafe { base.cast::<u8>().add(offset) };
    unsafe {
        copy_nonoverlapping(key.cast::<u8>(), slot, element_size);
        write_volatile(element_count, count + 1);
    }
    slot.cast()
}

#[no_mangle]
pub unsafe extern "C" fn insque(element: *mut c_void, predecessor: *mut c_void) {
    if element.is_null() {
        return;
    }
    let element_links = element.cast::<*mut c_void>();
    if predecessor.is_null() {
        unsafe {
            element_links.write(element);
            element_links.add(1).write(element);
        }
        return;
    }

    let predecessor_links = predecessor.cast::<*mut c_void>();
    let successor = unsafe { read_volatile(predecessor_links) };
    unsafe {
        element_links.write(successor);
        element_links.add(1).write(predecessor);
        write_volatile(predecessor_links, element);
        if !successor.is_null() {
            write_volatile(successor.cast::<*mut c_void>().add(1), element);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn remque(element: *mut c_void) {
    if element.is_null() {
        return;
    }
    let links = element.cast::<*mut c_void>();
    let successor = unsafe { read_volatile(links) };
    let predecessor = unsafe { read_volatile(links.add(1)) };
    if !predecessor.is_null() {
        unsafe { write_volatile(predecessor.cast::<*mut c_void>(), successor) };
    }
    if !successor.is_null() {
        unsafe { write_volatile(successor.cast::<*mut c_void>().add(1), predecessor) };
    }
}

// FIXME: Kernel runtime has no shared search-table allocator or persistent tree storage.
#[no_mangle]
pub extern "C" fn hcreate(_table_size: usize) -> c_int {
    set_errno(ENOSYS);
    0
}

#[no_mangle]
pub extern "C" fn hdestroy() {}

#[no_mangle]
pub unsafe extern "C" fn hsearch(_item: Entry, _action: c_int) -> *mut Entry {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn tdelete(
    _key: *const c_void,
    _root: *mut *mut c_void,
    _compare: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> c_int>,
) -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn tfind(
    _key: *const c_void,
    _root: *const *mut c_void,
    _compare: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> c_int>,
) -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn tsearch(
    _key: *const c_void,
    _root: *mut *mut c_void,
    _compare: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> c_int>,
) -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn twalk(
    _root: *const c_void,
    _action: Option<unsafe extern "C" fn(*const c_void, c_int, c_int)>,
) {}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}