/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::ffi::{c_int, c_void};
use core::mem::size_of;
use core::ptr;

use crate::kernel::error::Error;
use crate::kernel::net::wire::Ipv4Address;
use crate::kernel::runtime::runtime_libc::network;

use super::super::errno::ENOSYS;

const AF_INET: c_int = 2;
const SOCK_NONBLOCK: c_int = 0x0000_0800;
const SOCK_CLOEXEC: c_int = 0x0008_0000;
const MSG_NOSIGNAL: c_int = 0x4000;
const MSG_SUPPORTED: c_int = MSG_NOSIGNAL;
const MAX_IOV_COUNT: usize = 1024;
const MAX_MESSAGE_LENGTH: usize = 65_535;

#[repr(C)]
#[derive(Clone, Copy)]
struct SockaddrIn {
    family: u16,
    port: u16,
    address: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Iovec {
    base: *mut c_void,
    length: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Msghdr {
    name: *mut c_void,
    name_length: u32,
    vectors: *mut Iovec,
    vector_count: c_int,
    control: *mut c_void,
    control_length: u32,
    flags: c_int,
}

fn set_errno(error: Error) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(-(error as i32));
    }
}

fn fail(error: Error) -> c_int {
    set_errno(error);
    -1
}

fn fail_io(error: Error) -> isize {
    fail(error) as isize
}

unsafe fn read_sockaddr(
    address: *const c_void,
    address_length: u32,
) -> Result<(Ipv4Address, u16), Error> {
    if address.is_null() {
        return Err(Error::Fault);
    }
    if usize::try_from(address_length).map_err(|_| Error::Inval)? < size_of::<SockaddrIn>() {
        return Err(Error::Inval);
    }
    let address = unsafe { ptr::read_unaligned(address.cast::<SockaddrIn>()) };
    if i32::from(address.family) != AF_INET {
        return Err(Error::AfNoSupport);
    }
    Ok((
        Ipv4Address(address.address.to_ne_bytes()),
        u16::from_be(address.port),
    ))
}

unsafe fn write_sockaddr(
    output: *mut c_void,
    output_length: *mut u32,
    address: Ipv4Address,
    port: u16,
) -> Result<(), Error> {
    if output.is_null() && output_length.is_null() {
        return Ok(());
    }
    if output.is_null() || output_length.is_null() {
        return Err(Error::Fault);
    }
    let available = unsafe { ptr::read_unaligned(output_length) } as usize;
    if available < size_of::<SockaddrIn>() {
        unsafe { ptr::write_unaligned(output_length, size_of::<SockaddrIn>() as u32) };
        return Err(Error::MsgSize);
    }
    let sockaddr = SockaddrIn {
        family: AF_INET as u16,
        port: port.to_be(),
        address: u32::from_ne_bytes(address.0),
    };
    unsafe {
        ptr::write_unaligned(output.cast::<SockaddrIn>(), sockaddr);
        ptr::write_unaligned(output_length, size_of::<SockaddrIn>() as u32);
    }
    Ok(())
}

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
    socket: c_int,
    address: *mut c_void,
    address_length: *mut u32,
) -> c_int {
    match network::accept(socket) {
        Ok((accepted, peer_address, peer_port)) => {
            if let Err(error) = unsafe {
                write_sockaddr(address, address_length, peer_address, peer_port)
            } {
                let _ = network::close(accepted);
                return fail(error);
            }
            accepted
        }
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn accept4(
    socket: c_int,
    address: *mut c_void,
    address_length: *mut u32,
    flags: c_int,
) -> c_int {
    if flags & !(SOCK_NONBLOCK | SOCK_CLOEXEC) != 0 {
        return fail(Error::Inval);
    }
    let accepted = match unsafe { accept(socket, address, address_length) } {
        descriptor if descriptor >= 0 => descriptor,
        descriptor => return descriptor,
    };
    if flags & SOCK_NONBLOCK != 0 {
        if let Err(error) = network::set_nonblocking(accepted, true) {
            let _ = network::close(accepted);
            return fail(error);
        }
    }
    accepted
}

#[no_mangle]
pub unsafe extern "C" fn bind(
    socket: c_int,
    address: *const c_void,
    address_length: u32,
) -> c_int {
    let (address, port) = match unsafe { read_sockaddr(address, address_length) } {
        Ok(endpoint) => endpoint,
        Err(error) => return fail(error),
    };
    match network::bind(socket, address, port) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn connect(
    socket: c_int,
    address: *const c_void,
    address_length: u32,
) -> c_int {
    let (address, port) = match unsafe { read_sockaddr(address, address_length) } {
        Ok(endpoint) => endpoint,
        Err(error) => return fail(error),
    };
    match network::connect(socket, address, port) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn getpeername(
    socket: c_int,
    address: *mut c_void,
    address_length: *mut u32,
) -> c_int {
    let (address_value, port) = match network::endpoint(socket, true) {
        Ok(endpoint) => endpoint,
        Err(error) => return fail(error),
    };
    match unsafe { write_sockaddr(address, address_length, address_value, port) } {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn getsockname(
    socket: c_int,
    address: *mut c_void,
    address_length: *mut u32,
) -> c_int {
    let (address_value, port) = match network::endpoint(socket, false) {
        Ok(endpoint) => endpoint,
        Err(error) => return fail(error),
    };
    match unsafe { write_sockaddr(address, address_length, address_value, port) } {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn getsockopt(
    socket: c_int,
    level: c_int,
    option_name: c_int,
    option_value: *mut c_void,
    option_length: *mut u32,
) -> c_int {
    if option_value.is_null() || option_length.is_null() {
        return fail(Error::Fault);
    }
    if unsafe { ptr::read_unaligned(option_length) } < size_of::<c_int>() as u32 {
        unsafe { ptr::write_unaligned(option_length, size_of::<c_int>() as u32) };
        return fail(Error::MsgSize);
    }
    match network::get_socket_option(socket, level, option_name) {
        Ok(value) => {
            unsafe {
                ptr::write_unaligned(option_value.cast::<c_int>(), value);
                ptr::write_unaligned(option_length, size_of::<c_int>() as u32);
            }
            0
        }
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn listen(_socket: c_int, _backlog: c_int) -> c_int {
    match network::listen(_socket, _backlog) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn recv(
    socket: c_int,
    buffer: *mut c_void,
    length: usize,
    flags: c_int,
) -> isize {
    if length > isize::MAX as usize
        || flags & !MSG_SUPPORTED != 0
        || (buffer.is_null() && length != 0)
    {
        return fail_io(Error::Inval);
    }
    let output = if length == 0 {
        &mut [][..]
    } else {
        unsafe { core::slice::from_raw_parts_mut(buffer.cast::<u8>(), length) }
    };
    match network::receive(socket, output) {
        Ok(amount) => match isize::try_from(amount) {
            Ok(amount) => amount,
            Err(_) => fail_io(Error::Inval),
        },
        Err(error) => fail_io(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn recvfrom(
    socket: c_int,
    buffer: *mut c_void,
    length: usize,
    flags: c_int,
    address: *mut c_void,
    address_length: *mut u32,
) -> isize {
    if (!address.is_null() && address_length.is_null())
        || (address.is_null() && !address_length.is_null())
    {
        return fail_io(Error::Fault);
    }
    if length > isize::MAX as usize
        || flags & !MSG_SUPPORTED != 0
        || (buffer.is_null() && length != 0)
    {
        return fail_io(Error::Inval);
    }
    let output = if length == 0 {
        &mut [][..]
    } else {
        unsafe { core::slice::from_raw_parts_mut(buffer.cast::<u8>(), length) }
    };
    let (received, source) = match network::receive_from(socket, output) {
        Ok(result) => result,
        Err(error) => return fail_io(error),
    };
    if !address.is_null() {
        let (peer_address, peer_port) = match source.or_else(|| network::endpoint(socket, true).ok()) {
            Some(endpoint) => endpoint,
            None => return fail_io(Error::NotConn),
        };
        if let Err(error) = unsafe {
            write_sockaddr(address, address_length, peer_address, peer_port)
        } {
            return fail_io(error);
        }
    }
    match isize::try_from(received) {
        Ok(received) => received,
        Err(_) => fail_io(Error::Inval),
    }
}

#[no_mangle]
pub unsafe extern "C" fn recvmsg(
    socket: c_int,
    message: *mut c_void,
    flags: c_int,
) -> isize {
    if message.is_null() || flags & !MSG_SUPPORTED != 0 {
        return fail_io(if message.is_null() { Error::Fault } else { Error::Inval });
    }
    let mut header = unsafe { ptr::read_unaligned(message.cast::<Msghdr>()) };
    if header.control_length != 0 {
        return fail_io(Error::NoSys);
    }
    let vectors = match unsafe { message_vectors(&header) } {
        Ok(vectors) => vectors,
        Err(error) => return fail_io(error),
    };
    let total_length = match iov_total_length(vectors) {
        Ok(length) => length,
        Err(error) => return fail_io(error),
    };
    let mut payload = Vec::new();
    if payload.try_reserve_exact(total_length).is_err() {
        return fail_io(Error::NoMem);
    }
    payload.resize(total_length, 0);
    let (received, source) = match network::receive_from(socket, &mut payload) {
        Ok(result) => result,
        Err(error) => return fail_io(error),
    };
    let mut copied = 0;
    for vector in vectors {
        let length = vector.length.min(received.saturating_sub(copied));
        if length != 0 {
            unsafe {
                ptr::copy_nonoverlapping(
                    payload.as_ptr().add(copied),
                    vector.base.cast::<u8>(),
                    length,
                );
            }
            copied += length;
        }
    }
    header.flags = 0;
    if !header.name.is_null() {
        let peer = source.or_else(|| network::endpoint(socket, true).ok());
        let Some((address, port)) = peer else {
            return fail_io(Error::NotConn);
        };
        if let Err(error) = unsafe {
            write_sockaddr(header.name, &mut header.name_length, address, port)
        } {
            unsafe { ptr::write_unaligned(message.cast::<Msghdr>(), header) };
            return fail_io(error);
        }
    }
    unsafe { ptr::write_unaligned(message.cast::<Msghdr>(), header) };
    received as isize
}

#[no_mangle]
pub unsafe extern "C" fn send(
    socket: c_int,
    buffer: *const c_void,
    length: usize,
    flags: c_int,
) -> isize {
    if flags & !MSG_SUPPORTED != 0 || (buffer.is_null() && length != 0) {
        return fail_io(Error::Inval);
    }
    let input = if length == 0 {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(buffer.cast::<u8>(), length) }
    };
    match network::send(socket, input) {
        Ok(amount) => amount as isize,
        Err(error) => fail_io(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn sendmsg(
    socket: c_int,
    message: *const c_void,
    flags: c_int,
) -> isize {
    if message.is_null() || flags & !MSG_SUPPORTED != 0 {
        return fail_io(if message.is_null() { Error::Fault } else { Error::Inval });
    }
    let header = unsafe { ptr::read_unaligned(message.cast::<Msghdr>()) };
    if header.control_length != 0 {
        return fail_io(Error::NoSys);
    }
    let vectors = match unsafe { message_vectors(&header) } {
        Ok(vectors) => vectors,
        Err(error) => return fail_io(error),
    };
    let total_length = match iov_total_length(vectors) {
        Ok(length) => length,
        Err(error) => return fail_io(error),
    };
    let mut payload = Vec::new();
    if payload.try_reserve_exact(total_length).is_err() {
        return fail_io(Error::NoMem);
    }
    for vector in vectors {
        if vector.length != 0 {
            let bytes = unsafe {
                core::slice::from_raw_parts(vector.base.cast::<u8>(), vector.length)
            };
            payload.extend_from_slice(bytes);
        }
    }
    let destination = if header.name.is_null() {
        None
    } else {
        match unsafe { read_sockaddr(header.name, header.name_length) } {
            Ok(endpoint) => Some(endpoint),
            Err(error) => return fail_io(error),
        }
    };
    match network::send_to(socket, destination, &payload) {
        Ok(amount) => amount as isize,
        Err(error) => fail_io(error),
    }
}

unsafe fn message_vectors<'a>(header: &Msghdr) -> Result<&'a [Iovec], Error> {
    let count = usize::try_from(header.vector_count).map_err(|_| Error::Inval)?;
    if count > MAX_IOV_COUNT {
        return Err(Error::Inval);
    }
    if count == 0 {
        return Ok(&[]);
    }
    if header.vectors.is_null() {
        return Err(Error::Fault);
    }
    Ok(unsafe { core::slice::from_raw_parts(header.vectors, count) })
}

fn iov_total_length(vectors: &[Iovec]) -> Result<usize, Error> {
    let mut total = 0usize;
    for vector in vectors {
        if vector.base.is_null() && vector.length != 0 {
            return Err(Error::Fault);
        }
        total = total.checked_add(vector.length).ok_or(Error::Overflow)?;
        if total > MAX_MESSAGE_LENGTH {
            return Err(Error::TooBig);
        }
    }
    Ok(total)
}

#[no_mangle]
pub unsafe extern "C" fn sendto(
    socket: c_int,
    buffer: *const c_void,
    length: usize,
    flags: c_int,
    address: *const c_void,
    address_length: u32,
) -> isize {
    if flags & !MSG_SUPPORTED != 0 || (buffer.is_null() && length != 0) {
        return fail_io(Error::Inval);
    }
    let destination = if address.is_null() {
        None
    } else {
        match unsafe { read_sockaddr(address, address_length) } {
            Ok(endpoint) => Some(endpoint),
            Err(error) => return fail_io(error),
        }
    };
    let input = if length == 0 {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(buffer.cast::<u8>(), length) }
    };
    match network::send_to(socket, destination, input) {
        Ok(amount) => amount as isize,
        Err(error) => fail_io(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn setsockopt(
    socket: c_int,
    level: c_int,
    option_name: c_int,
    option_value: *const c_void,
    option_length: u32,
) -> c_int {
    if option_value.is_null() || option_length < size_of::<c_int>() as u32 {
        return fail(if option_value.is_null() {
            Error::Fault
        } else {
            Error::Inval
        });
    }
    let value = unsafe { ptr::read_unaligned(option_value.cast::<c_int>()) };
    match network::set_socket_option(socket, level, option_name, value) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn shutdown(socket: c_int, how: c_int) -> c_int {
    match network::shutdown(socket, how) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn sockatmark(_socket: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn socket(domain: c_int, socket_type: c_int, protocol: c_int) -> c_int {
    match network::create_socket(domain, socket_type, protocol) {
        Ok(descriptor) => descriptor,
        Err(error) => fail(error),
    }
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