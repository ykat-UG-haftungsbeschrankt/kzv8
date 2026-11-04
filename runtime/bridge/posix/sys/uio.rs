use core::ffi::{c_int, c_void};
use core::ptr::read_volatile;

const EFAULT: c_int = 14;
const EINVAL: c_int = 22;
const IOV_MAX: c_int = 16;

#[repr(C)]
#[derive(Clone, Copy)]
struct Iovec {
    base: *mut c_void,
    length: usize,
}

unsafe extern "C" {
    fn read(file_descriptor: c_int, buffer: *mut c_void, length: usize) -> isize;
    fn write(file_descriptor: c_int, buffer: *const c_void, length: usize) -> isize;
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub unsafe extern "C" fn readv(
    file_descriptor: c_int,
    vectors: *const Iovec,
    count: c_int,
) -> isize {
    unsafe { vector_io(file_descriptor, vectors, count, false) }
}

#[no_mangle]
pub unsafe extern "C" fn writev(
    file_descriptor: c_int,
    vectors: *const Iovec,
    count: c_int,
) -> isize {
    unsafe { vector_io(file_descriptor, vectors, count, true) }
}

unsafe fn vector_io(
    file_descriptor: c_int,
    vectors: *const Iovec,
    count: c_int,
    writing: bool,
) -> isize {
    if count < 0 || count > IOV_MAX {
        set_errno(EINVAL);
        return -1;
    }
    if count == 0 {
        return 0;
    }
    if vectors.is_null() {
        set_errno(EFAULT);
        return -1;
    }

    let mut total_length = 0usize;
    for index in 0..count as usize {
        let vector = unsafe { read_volatile(vectors.add(index)) };
        if vector.base.is_null() && vector.length != 0 {
            set_errno(EFAULT);
            return -1;
        }
        let Some(next_total) = total_length.checked_add(vector.length) else {
            set_errno(EINVAL);
            return -1;
        };
        if next_total > isize::MAX as usize {
            set_errno(EINVAL);
            return -1;
        }
        total_length = next_total;
    }

    let mut transferred = 0isize;
    for index in 0..count as usize {
        let vector = unsafe { read_volatile(vectors.add(index)) };
        if vector.length == 0 {
            continue;
        }
        let amount = unsafe {
            if writing {
                write(file_descriptor, vector.base.cast_const(), vector.length)
            } else {
                read(file_descriptor, vector.base, vector.length)
            }
        };
        if amount < 0 {
            return if transferred == 0 { -1 } else { transferred };
        }
        transferred += amount;
        if amount as usize != vector.length {
            break;
        }
    }
    transferred
}