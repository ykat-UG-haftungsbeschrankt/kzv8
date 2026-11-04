use core::ffi::{c_int, c_void};
use super::super::errno::ENOSYS;

// FIXME: Kernel TCP handles are event-loop-owned; there is no POSIX socket descriptor table,
// and UDP and UNIX-domain socket backends are not available.
fn unavailable() -> c_int {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    -1
}

fn unavailable_io() -> isize {
    unavailable() as isize
}

#[no_mangle]
pub unsafe extern "C" fn accept(
    _socket: c_int,
    _address: *mut c_void,
    _address_length: *mut u32,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn accept4(
    _socket: c_int,
    _address: *mut c_void,
    _address_length: *mut u32,
    _flags: c_int,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn bind(
    _socket: c_int,
    _address: *const c_void,
    _address_length: u32,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn connect(
    _socket: c_int,
    _address: *const c_void,
    _address_length: u32,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn getpeername(
    _socket: c_int,
    _address: *mut c_void,
    _address_length: *mut u32,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn getsockname(
    _socket: c_int,
    _address: *mut c_void,
    _address_length: *mut u32,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn getsockopt(
    _socket: c_int,
    _level: c_int,
    _option_name: c_int,
    _option_value: *mut c_void,
    _option_length: *mut u32,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn listen(_socket: c_int, _backlog: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn recv(
    _socket: c_int,
    _buffer: *mut c_void,
    _length: usize,
    _flags: c_int,
) -> isize {
    unavailable_io()
}

#[no_mangle]
pub unsafe extern "C" fn recvfrom(
    _socket: c_int,
    _buffer: *mut c_void,
    _length: usize,
    _flags: c_int,
    _address: *mut c_void,
    _address_length: *mut u32,
) -> isize {
    unavailable_io()
}

#[no_mangle]
pub unsafe extern "C" fn recvmsg(
    _socket: c_int,
    _message: *mut c_void,
    _flags: c_int,
) -> isize {
    unavailable_io()
}

#[no_mangle]
pub unsafe extern "C" fn send(
    _socket: c_int,
    _buffer: *const c_void,
    _length: usize,
    _flags: c_int,
) -> isize {
    unavailable_io()
}

#[no_mangle]
pub unsafe extern "C" fn sendmsg(
    _socket: c_int,
    _message: *const c_void,
    _flags: c_int,
) -> isize {
    unavailable_io()
}

#[no_mangle]
pub unsafe extern "C" fn sendto(
    _socket: c_int,
    _buffer: *const c_void,
    _length: usize,
    _flags: c_int,
    _address: *const c_void,
    _address_length: u32,
) -> isize {
    unavailable_io()
}

#[no_mangle]
pub unsafe extern "C" fn setsockopt(
    _socket: c_int,
    _level: c_int,
    _option_name: c_int,
    _option_value: *const c_void,
    _option_length: u32,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn shutdown(_socket: c_int, _how: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn sockatmark(_socket: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn socket(_domain: c_int, _type: c_int, _protocol: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn socketpair(
    _domain: c_int,
    _type: c_int,
    _protocol: c_int,
    _sockets: *mut c_int,
) -> c_int {
    unavailable()
}