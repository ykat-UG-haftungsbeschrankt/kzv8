use core::ffi::{c_char, c_int, c_void};
use super::errno::ENOSYS;

const EAI_AGAIN: c_int = -1;
const EAI_BADFLAGS: c_int = -2;
const EAI_FAIL: c_int = -3;
const EAI_FAMILY: c_int = -4;
const EAI_MEMORY: c_int = -5;
const EAI_NONAME: c_int = -6;
const EAI_SERVICE: c_int = -7;
const EAI_SOCKTYPE: c_int = -8;
const EAI_SYSTEM: c_int = -9;
const EAI_OVERFLOW: c_int = -10;

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

fn gai_message(error: c_int) -> *const c_char {
    let message: &'static [u8] = match error {
        EAI_AGAIN => b"temporary failure in name resolution\0",
        EAI_BADFLAGS => b"invalid flags\0",
        EAI_FAIL => b"non-recoverable name resolution failure\0",
        EAI_FAMILY => b"address family not supported\0",
        EAI_MEMORY => b"memory allocation failure\0",
        EAI_NONAME => b"name or service not known\0",
        EAI_SERVICE => b"service not supported for socket type\0",
        EAI_SOCKTYPE => b"socket type not supported\0",
        EAI_SYSTEM => b"system error\0",
        EAI_OVERFLOW => b"argument buffer overflow\0",
        _ => b"unknown name resolution error\0",
    };
    message.as_ptr().cast()
}

// FIXME: The kernel has no DNS resolver or host, network, protocol, and service databases.
#[no_mangle]
pub unsafe extern "C" fn endhostent() {}

#[no_mangle]
pub unsafe extern "C" fn endnetent() {}

#[no_mangle]
pub unsafe extern "C" fn endprotoent() {}

#[no_mangle]
pub unsafe extern "C" fn endservent() {}

#[no_mangle]
pub unsafe extern "C" fn freeaddrinfo(_results: *mut c_void) {}

#[no_mangle]
pub extern "C" fn gai_strerror(error_code: c_int) -> *const c_char {
    gai_message(error_code)
}

#[no_mangle]
pub unsafe extern "C" fn getaddrinfo(
    _node: *const c_char,
    _service: *const c_char,
    _hints: *const c_void,
    results: *mut *mut c_void,
) -> c_int {
    if !results.is_null() {
        unsafe { results.write(core::ptr::null_mut()) };
    }
    set_errno(ENOSYS);
    EAI_SYSTEM
}

#[no_mangle]
pub unsafe extern "C" fn gethostent() -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getnameinfo(
    _address: *const c_void,
    _address_length: u32,
    _host: *mut c_char,
    _host_length: u32,
    _service: *mut c_char,
    _service_length: u32,
    _flags: c_int,
) -> c_int {
    set_errno(ENOSYS);
    EAI_SYSTEM
}

#[no_mangle]
pub unsafe extern "C" fn getnetbyaddr(_network: u32, _address_type: c_int) -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getnetbyname(_name: *const c_char) -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getnetent() -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getprotobyname(_name: *const c_char) -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getprotobynumber(_protocol: c_int) -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getprotoent() -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getservbyname(
    _name: *const c_char,
    _protocol: *const c_char,
) -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getservbyport(
    _port: c_int,
    _protocol: *const c_char,
) -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getservent() -> *mut c_void {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn sethostent(_stay_open: c_int) {}

#[no_mangle]
pub unsafe extern "C" fn setnetent(_stay_open: c_int) {}

#[no_mangle]
pub unsafe extern "C" fn setprotoent(_stay_open: c_int) {}

#[no_mangle]
pub unsafe extern "C" fn setservent(_stay_open: c_int) {}