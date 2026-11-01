use core::ffi::c_void;

const PAGE_SIZE: usize = 4096;

const EINVAL: i32 = 22;
const ENOMEM: i32 = 12;
const MADV_WILLNEED: i32 = 3;
const MADV_DONTNEED: i32 = 4;
const MADV_FREE: i32 = 8;

const PROT_READ: i32 = 1;
const PROT_WRITE: i32 = 2;
const PROT_EXEC: i32 = 4;
const PROT_READ_WRITE: i32 = PROT_READ | PROT_WRITE;
const PROT_READ_EXEC: i32 = PROT_READ | PROT_EXEC;
const PROT_WRITE_EXEC: i32 = PROT_WRITE | PROT_EXEC;
const PROT_READ_WRITE_EXEC: i32 = PROT_READ | PROT_WRITE | PROT_EXEC;

const MAP_SHARED: i32 = 1;
const MAP_PRIVATE: i32 = 2;
const MAP_ANONYMOUS: i32 = 0x20;

fn set_errno(error: i32) {
    unsafe { crate::kernel_thread::__errno_location().write(error) };
}

fn fail(error: i32) -> *mut c_void {
    set_errno(error);
    usize::MAX as *mut c_void
}

fn rounded_length(length: usize) -> Option<usize> {
    length
        .checked_add(PAGE_SIZE - 1)
        .map(|value| value & !(PAGE_SIZE - 1))
}

fn page_permissions(protection: i32) -> Option<i32> {
    match protection {
        0 => Some(0),
        PROT_READ => Some(1),
        PROT_WRITE | PROT_READ_WRITE => Some(2),
        PROT_EXEC | PROT_READ_EXEC => Some(4),
        PROT_WRITE_EXEC | PROT_READ_WRITE_EXEC => Some(3),
        _ => None,
    }
}

#[no_mangle]
pub unsafe extern "C" fn mmap(
    _address_hint: *mut c_void,
    length: usize,
    protection: i32,
    flags: i32,
    file_descriptor: i32,
    offset: i64,
) -> *mut c_void {
    let sharing = flags & (MAP_PRIVATE | MAP_SHARED);
    let supported_flags = MAP_PRIVATE | MAP_SHARED | MAP_ANONYMOUS;
    if length == 0
        || protection & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0
        || flags & !supported_flags != 0
        || flags & MAP_ANONYMOUS == 0
        || !matches!(sharing, MAP_PRIVATE | MAP_SHARED)
        || file_descriptor != -1
        || offset != 0
    {
        return fail(EINVAL);
    }
    let Some(permissions) = page_permissions(protection) else {
        return fail(EINVAL);
    };
    let Some(length) = rounded_length(length) else {
        return fail(ENOMEM);
    };
    let Some(address) = crate::memory::reserve(length, PAGE_SIZE, permissions) else {
        return fail(ENOMEM);
    };
    address as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn munmap(address: *mut c_void, length: usize) -> i32 {
    let address = address as usize;
    if address == 0 || address % PAGE_SIZE != 0 || length == 0 {
        set_errno(EINVAL);
        return -1;
    }
    let Some(length) = rounded_length(length) else {
        set_errno(EINVAL);
        return -1;
    };
    if !crate::memory::release(address, length) {
        set_errno(EINVAL);
        return -1;
    }
    0
}
