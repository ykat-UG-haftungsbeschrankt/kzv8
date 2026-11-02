use core::ffi::c_void;
use core::mem::{align_of, size_of};
use core::ptr::null_mut;

const EINVAL: i32 = 22;
const ENOMEM: i32 = 12;

fn set_errno(error: i32) {
    unsafe { crate::kernel_thread::__errno_location().write(error) };
}

#[no_mangle]
pub extern "C" fn malloc(size: usize) -> *mut c_void {
    let pointer = crate::heap::allocate_c_layout(size, align_of::<u128>());
    if pointer.is_null() {
        set_errno(ENOMEM);
    }
    pointer
}

#[no_mangle]
pub extern "C" fn aligned_alloc(alignment: usize, size: usize) -> *mut c_void {
    if alignment < align_of::<usize>() || !alignment.is_power_of_two() || size % alignment != 0 {
        set_errno(EINVAL);
        return null_mut();
    }
    let pointer = crate::heap::allocate_c_layout(size, alignment);
    if pointer.is_null() {
        set_errno(ENOMEM);
    }
    pointer
}

#[no_mangle]
pub unsafe extern "C" fn posix_memalign(
    output: *mut *mut c_void,
    alignment: usize,
    size: usize,
) -> i32 {
    if output.is_null()
        || alignment < size_of::<*mut c_void>()
        || alignment % size_of::<*mut c_void>() != 0
        || !alignment.is_power_of_two()
    {
        return EINVAL;
    }
    let pointer = crate::heap::allocate_c_layout(size, alignment);
    if pointer.is_null() {
        return ENOMEM;
    }
    unsafe { output.write(pointer) };
    0
}

#[no_mangle]
pub extern "C" fn free(pointer: *mut c_void) {
    crate::heap::deallocate_c_pointer(pointer);
}

#[no_mangle]
pub extern "C" fn malloc_usable_size(pointer: *mut c_void) -> usize {
    crate::heap::c_allocation_capacity(pointer.cast::<u8>()).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn calloc(count: usize, size: usize) -> *mut c_void {
    let Some(total) = count.checked_mul(size) else {
        return null_mut();
    };
    let pointer = malloc(total);
    if !pointer.is_null() {
        unsafe { pointer.cast::<u8>().write_bytes(0, total) };
    }
    pointer
}

#[no_mangle]
pub unsafe extern "C" fn realloc(pointer: *mut c_void, size: usize) -> *mut c_void {
    if pointer.is_null() {
        return malloc(size);
    }
    if size == 0 {
        free(pointer);
        return null_mut();
    }

    let Some(capacity) = crate::heap::c_allocation_capacity(pointer.cast::<u8>()) else {
        return null_mut();
    };
    let replacement = malloc(size);
    if replacement.is_null() {
        return null_mut();
    }
    unsafe {
        core::ptr::copy_nonoverlapping(
            pointer.cast::<u8>(),
            replacement.cast::<u8>(),
            capacity.min(size),
        );
    }
    free(pointer);
    replacement
}
