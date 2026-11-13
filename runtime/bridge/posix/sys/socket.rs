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
use crate::kernel::dl::AddressSpace;
use crate::kernel::net::wire::Ipv4Address;
use crate::kernel::runtime::runtime_libc::network;
use crate::kernel::scheduler::thread::ThreadManager;

use super::super::errno::ENOSYS;

const AF_INET: c_int = 2;
const AF_UNIX: c_int = 1;
const SOCK_NONBLOCK: c_int = 0x0000_0800;
const SOCK_CLOEXEC: c_int = 0x0008_0000;
const SOCK_CLOFORK: c_int = 0x0010_0000;
const SOL_SOCKET: c_int = 1;
const SCM_RIGHTS: c_int = 1;
const SCM_CREDENTIALS: c_int = 2;
const SO_PASSCRED: c_int = 16;
const MSG_OOB: c_int = 0x0001;
const MSG_PEEK: c_int = 0x0002;
const MSG_CTRUNC: c_int = 0x0008;
const MSG_NOSIGNAL: c_int = 0x4000;
const MSG_CMSG_CLOEXEC: c_int = 0x4000_0000;
const MSG_SUPPORTED: c_int = MSG_NOSIGNAL | MSG_OOB | MSG_PEEK | MSG_CMSG_CLOEXEC;
const MAX_IOV_COUNT: usize = 1024;
const MAX_MESSAGE_LENGTH: usize = 65_535;
const MAX_CONTROL_LENGTH: usize = 65_536;
const F_SETFD: c_int = 2;
const FD_CLOEXEC: usize = 1;

unsafe extern "C" {
    fn dup(file_descriptor: c_int) -> c_int;
    fn close(file_descriptor: c_int) -> c_int;
    fn fcntl(file_descriptor: c_int, command: c_int, argument: usize) -> c_int;
}

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

#[repr(C)]
#[derive(Clone, Copy)]
struct CmsgHdr {
    length: u32,
    level: c_int,
    kind: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Ucred {
    process_id: i32,
    user_id: u32,
    group_id: u32,
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

unsafe fn read_socket_family(address: *const c_void, address_length: u32) -> Result<u16, Error> {
    if address.is_null() {
        return Err(Error::Fault);
    }
    if usize::try_from(address_length).map_err(|_| Error::Inval)? < size_of::<u16>() {
        return Err(Error::Inval);
    }
    let process = ThreadManager::current_process()?;
    let mut family = [0; size_of::<u16>()];
    process.address_space().read(address as usize, &mut family)?;
    Ok(u16::from_ne_bytes(family))
}

unsafe fn read_local_sockaddr(
    address: *const c_void,
    address_length: u32,
) -> Result<Vec<u8>, Error> {
    if unsafe { read_socket_family(address, address_length) }? != AF_UNIX as u16 {
        return Err(Error::AfNoSupport);
    }
    let address_length = usize::try_from(address_length).map_err(|_| Error::Inval)?;
    let path_length = address_length
        .saturating_sub(size_of::<u16>())
        .min(108);
    if path_length == 0 {
        return Err(Error::Inval);
    }
    let process = ThreadManager::current_process()?;
    let mut path_bytes = [0; 108];
    process.address_space().read(
        (address as usize)
            .checked_add(size_of::<u16>())
            .ok_or(Error::Fault)?,
        &mut path_bytes[..path_length],
    )?;
    if path_bytes[0] == 0 {
        return Err(Error::OpNotSupp);
    }
    let path_length = path_bytes[..path_length]
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(path_length);
    if path_length == 0 || path_length > 107 {
        return Err(Error::NameTooLong);
    }
    let mut path = Vec::new();
    path.try_reserve_exact(path_length).map_err(|_| Error::NoMem)?;
    path.extend_from_slice(&path_bytes[..path_length]);
    Ok(path)
}

fn cmsg_aligned_length(length: usize) -> Result<usize, Error> {
    let alignment = size_of::<usize>();
    length
        .checked_add(alignment - 1)
        .map(|length| length & !(alignment - 1))
        .ok_or(Error::Overflow)
}

fn close_control_rights(rights: Vec<c_int>) {
    for descriptor in rights {
        let _ = unsafe { close(descriptor) };
    }
}

fn close_control_messages(messages: Vec<network::LocalControlMessage>) {
    for message in messages {
        close_control_rights(message.rights);
    }
}

fn duplicate_control_right(file_descriptor: c_int) -> Result<c_int, Error> {
    let duplicate = unsafe { dup(file_descriptor) };
    if duplicate < 0 {
        Err(Error::BadFd)
    } else {
        Ok(duplicate)
    }
}

fn validate_passed_credentials(credentials: Ucred) -> Result<(), Error> {
    let process = ThreadManager::current_process()?;
    if process.effective_user_is_root() {
        return Ok(());
    }
    let user_ids = process.user_ids();
    let group_ids = process.group_ids();
    if credentials.process_id != process.pid()
        || !user_ids.contains(&credentials.user_id)
        || !group_ids.contains(&credentials.group_id)
    {
        return Err(Error::Perm);
    }
    Ok(())
}

unsafe fn parse_control_messages(
    control: *const c_void,
    control_length: u32,
) -> Result<Option<network::LocalControlMessage>, Error> {
    let length = usize::try_from(control_length).map_err(|_| Error::TooBig)?;
    if length == 0 {
        return Ok(None);
    }
    if control.is_null() {
        return Err(Error::Fault);
    }
    if length > MAX_CONTROL_LENGTH {
        return Err(Error::TooBig);
    }
    let base = control.cast::<u8>();
    let data_offset = cmsg_aligned_length(size_of::<CmsgHdr>())?;
    let mut offset = 0usize;
    let mut rights = Vec::new();
    let mut credentials = None;
    while offset < length {
        let remaining = length - offset;
        if remaining < size_of::<CmsgHdr>() {
            break;
        }
        let header = unsafe { ptr::read_unaligned(base.add(offset).cast::<CmsgHdr>()) };
        let header_length = usize::try_from(header.length).map_err(|_| Error::Inval)?;
        if header_length < data_offset || header_length > remaining {
            close_control_rights(rights);
            return Err(Error::Inval);
        }
        let payload_length = header_length - data_offset;
        let payload = unsafe { base.add(offset + data_offset) };
        if header.level != SOL_SOCKET {
            close_control_rights(rights);
            return Err(Error::NoProtoOpt);
        }
        match header.kind {
            SCM_RIGHTS => {
                if payload_length == 0 || payload_length % size_of::<c_int>() != 0 {
                    close_control_rights(rights);
                    return Err(Error::Inval);
                }
                let descriptor_count = payload_length / size_of::<c_int>();
                if rights.try_reserve(descriptor_count).is_err() {
                    close_control_rights(rights);
                    return Err(Error::NoMem);
                }
                for index in 0..descriptor_count {
                    let source = unsafe {
                        ptr::read_unaligned(payload.add(index * size_of::<c_int>()).cast::<c_int>())
                    };
                    match duplicate_control_right(source) {
                        Ok(duplicate) => rights.push(duplicate),
                        Err(error) => {
                            close_control_rights(rights);
                            return Err(error);
                        }
                    }
                }
            }
            SCM_CREDENTIALS => {
                if payload_length != size_of::<Ucred>() || credentials.is_some() {
                    close_control_rights(rights);
                    return Err(Error::Inval);
                }
                let passed = unsafe { ptr::read_unaligned(payload.cast::<Ucred>()) };
                if let Err(error) = validate_passed_credentials(passed) {
                    close_control_rights(rights);
                    return Err(error);
                }
                credentials = Some(network::LocalCredentials {
                    process_id: passed.process_id,
                    user_id: passed.user_id,
                    group_id: passed.group_id,
                });
            }
            _ => {
                close_control_rights(rights);
                return Err(Error::NoProtoOpt);
            }
        }
        let aligned_length = cmsg_aligned_length(header_length)?;
        if aligned_length > remaining {
            if header_length != remaining {
                close_control_rights(rights);
                return Err(Error::Inval);
            }
            offset = length;
        } else {
            offset = offset.checked_add(aligned_length).ok_or(Error::Overflow)?;
        }
    }
    if rights.is_empty() && credentials.is_none() {
        return Ok(None);
    }
    Ok(Some(network::LocalControlMessage { rights, credentials }))
}

fn write_control_messages(
    header: &mut Msghdr,
    mut messages: Vec<network::LocalControlMessage>,
    peek: bool,
    close_on_exec: bool,
) -> Result<bool, Error> {
    let has_control = messages
        .iter()
        .any(|message| !message.rights.is_empty() || message.credentials.is_some());
    if !has_control {
        header.control_length = 0;
        return Ok(false);
    }
    let total_rights = messages.iter().map(|message| message.rights.len()).sum::<usize>();
    let mut rights = Vec::new();
    if rights.try_reserve_exact(total_rights).is_err() {
        if !peek {
            close_control_messages(messages);
        }
        return Err(Error::NoMem);
    }
    let credentials = messages.iter().find_map(|message| message.credentials);
    for message in &mut messages {
        rights.extend(message.rights.drain(..));
    }
    if peek {
        let mut duplicates = Vec::new();
        if duplicates.try_reserve_exact(rights.len()).is_err() {
            return Err(Error::NoMem);
        }
        for descriptor in rights {
            match duplicate_control_right(descriptor) {
                Ok(duplicate) => duplicates.push(duplicate),
                Err(error) => {
                    close_control_rights(duplicates);
                    return Err(error);
                }
            }
        }
        rights = duplicates;
    }

    let available = usize::try_from(header.control_length).map_err(|_| Error::Inval)?;
    let Some(control) = (!header.control.is_null() && available != 0).then_some(header.control.cast::<u8>()) else {
        close_control_rights(rights);
        header.control_length = 0;
        return Ok(true);
    };
    let mut offset = 0usize;
    let mut truncated = false;
    if !rights.is_empty() {
        let data_offset = cmsg_aligned_length(size_of::<CmsgHdr>())?;
        let remaining = available.saturating_sub(offset);
        let delivered_count = if remaining < data_offset + size_of::<c_int>() {
            0
        } else {
            rights.len().min((remaining - data_offset) / size_of::<c_int>())
        };
        if delivered_count == 0 {
            truncated = true;
            close_control_rights(rights);
        } else {
            let payload_length = delivered_count * size_of::<c_int>();
            let cmsg_length = data_offset + payload_length;
            let header_value = CmsgHdr {
                length: u32::try_from(cmsg_length).map_err(|_| Error::Overflow)?,
                level: SOL_SOCKET,
                kind: SCM_RIGHTS,
            };
            unsafe {
                ptr::write_unaligned(control.add(offset).cast::<CmsgHdr>(), header_value);
                for (index, descriptor) in rights.iter().take(delivered_count).enumerate() {
                    ptr::write_unaligned(
                        control
                            .add(offset + data_offset + index * size_of::<c_int>())
                            .cast::<c_int>(),
                        *descriptor,
                    );
                }
            }
            offset += cmsg_aligned_length(cmsg_length)?;
            if delivered_count < rights.len() {
                truncated = true;
                close_control_rights(rights.drain(delivered_count..).collect());
                rights.truncate(delivered_count);
            }
            if close_on_exec {
                for descriptor in &rights {
                    if unsafe { fcntl(*descriptor, F_SETFD, FD_CLOEXEC) } < 0 {
                        close_control_rights(rights);
                        header.control_length = 0;
                        return Err(Error::BadFd);
                    }
                }
            }
        }
    }
    if let Some(credentials) = credentials {
        let data_offset = cmsg_aligned_length(size_of::<CmsgHdr>())?;
        let cmsg_length = data_offset + size_of::<Ucred>();
        let aligned_length = cmsg_aligned_length(cmsg_length)?;
        if offset.checked_add(aligned_length).is_some_and(|end| end <= available) {
            let header_value = CmsgHdr {
                length: u32::try_from(cmsg_length).map_err(|_| Error::Overflow)?,
                level: SOL_SOCKET,
                kind: SCM_CREDENTIALS,
            };
            let credentials = Ucred {
                process_id: credentials.process_id,
                user_id: credentials.user_id,
                group_id: credentials.group_id,
            };
            unsafe {
                ptr::write_unaligned(control.add(offset).cast::<CmsgHdr>(), header_value);
                ptr::write_unaligned(
                    control.add(offset + data_offset).cast::<Ucred>(),
                    credentials,
                );
            }
            offset += aligned_length;
        } else {
            truncated = true;
        }
    }
    header.control_length = u32::try_from(offset.min(available)).map_err(|_| Error::Overflow)?;
    Ok(truncated)
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

unsafe fn write_local_sockaddr(
    output: *mut c_void,
    output_length: *mut u32,
    path: &[u8],
) -> Result<(), Error> {
    if output.is_null() && output_length.is_null() {
        return Ok(());
    }
    if output.is_null() || output_length.is_null() {
        return Err(Error::Fault);
    }
    if path.len() > 107 {
        return Err(Error::NameTooLong);
    }
    let required = size_of::<u16>() + if path.is_empty() { 0 } else { path.len() + 1 };
    let process = ThreadManager::current_process()?;
    let mut address_space = process.address_space();
    let mut length_bytes = [0; size_of::<u32>()];
    address_space.read(output_length as usize, &mut length_bytes)?;
    let available = u32::from_ne_bytes(length_bytes) as usize;
    if available < required {
        address_space.write(output_length as usize, &(required as u32).to_ne_bytes())?;
        return Err(Error::MsgSize);
    }
    let mut sockaddr = [0; 110];
    sockaddr[..2].copy_from_slice(&(AF_UNIX as u16).to_ne_bytes());
    if !path.is_empty() {
        sockaddr[2..2 + path.len()].copy_from_slice(path);
    }
    address_space.write(output as usize, &sockaddr[..required])?;
    address_space.write(output_length as usize, &(required as u32).to_ne_bytes())?;
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
            let address_result = if network::is_local_socket_fd(accepted) {
                network::local_socket_names(accepted, true).and_then(|peer_path| unsafe {
                    write_local_sockaddr(address, address_length, &peer_path)
                })
            } else {
                unsafe { write_sockaddr(address, address_length, peer_address, peer_port) }
            };
            if let Err(error) = address_result {
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
    if flags & SOCK_CLOEXEC != 0 {
        if let Err(error) = network::set_descriptor_flags(accepted, 1) {
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
    if matches!(unsafe { read_socket_family(address, address_length) }, Ok(family) if family == AF_UNIX as u16)
    {
        let path = match unsafe { read_local_sockaddr(address, address_length) } {
            Ok(path) => path,
            Err(error) => return fail(error),
        };
        let path = match crate::kernel::runtime::runtime_libc::filesystem::normalize_local_socket_path(&path) {
            Ok(path) => path,
            Err(error) => {
                unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
                return -1;
            }
        };
        if let Err(error) = crate::kernel::runtime::runtime_libc::filesystem::create_local_socket_node(&path) {
            unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
            return -1;
        }
        return match network::bind_local_socket(socket, &path) {
            Ok(()) => 0,
            Err(error) => {
                let _ = crate::kernel::runtime::runtime_libc::filesystem::remove_local_socket_node(&path);
                fail(error)
            }
        };
    }
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
    if matches!(unsafe { read_socket_family(address, address_length) }, Ok(family) if family == AF_UNIX as u16)
    {
        let path = match unsafe { read_local_sockaddr(address, address_length) } {
            Ok(path) => path,
            Err(error) => return fail(error),
        };
        let path = match crate::kernel::runtime::runtime_libc::filesystem::normalize_local_socket_path(&path) {
            Ok(path) => path,
            Err(error) => {
                unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
                return -1;
            }
        };
        if let Err(error) = crate::kernel::runtime::runtime_libc::filesystem::check_local_socket_connect(&path) {
            unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
            return -1;
        }
        return match network::connect_local_socket(socket, &path) {
            Ok(()) => 0,
            Err(error) => fail(error),
        };
    }
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
    if network::is_local_socket_fd(socket) {
        if let Err(error) = network::validate_local_socket(socket) {
            return fail(error);
        }
        let peer_path = match network::local_socket_names(socket, true) {
            Ok(path) => path,
            Err(error) => return fail(error),
        };
        return match unsafe { write_local_sockaddr(address, address_length, &peer_path) } {
            Ok(()) => 0,
            Err(error) => fail(error),
        };
    }
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
    if network::is_local_socket_fd(socket) {
        if let Err(error) = network::validate_local_socket(socket) {
            return fail(error);
        }
        let local_path = match network::local_socket_names(socket, false) {
            Ok(path) => path,
            Err(error) => return fail(error),
        };
        return match unsafe { write_local_sockaddr(address, address_length, &local_path) } {
            Ok(()) => 0,
            Err(error) => fail(error),
        };
    }
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
        || flags & !(MSG_NOSIGNAL | MSG_OOB | MSG_PEEK) != 0
        || (buffer.is_null() && length != 0)
    {
        return fail_io(Error::Inval);
    }
    if !network::is_local_socket_fd(socket) && flags & (MSG_OOB | MSG_PEEK) != 0 {
        return fail_io(Error::OpNotSupp);
    }
    let output = if length == 0 {
        &mut [][..]
    } else {
        unsafe { core::slice::from_raw_parts_mut(buffer.cast::<u8>(), length) }
    };
    if network::is_local_socket_fd(socket) {
        return match network::receive_from_local_socket_with_control(
            socket,
            output,
            flags & MSG_PEEK != 0,
            flags & MSG_OOB != 0,
            false,
        ) {
            Ok(result) => result.length as isize,
            Err(error) => fail_io(error),
        };
    }
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
        || flags & !(MSG_NOSIGNAL | MSG_OOB | MSG_PEEK) != 0
        || (buffer.is_null() && length != 0)
    {
        return fail_io(Error::Inval);
    }
    if !network::is_local_socket_fd(socket) && flags & (MSG_OOB | MSG_PEEK) != 0 {
        return fail_io(Error::OpNotSupp);
    }
    let output = if length == 0 {
        &mut [][..]
    } else {
        unsafe { core::slice::from_raw_parts_mut(buffer.cast::<u8>(), length) }
    };
    let (received, source) = if network::is_local_socket_fd(socket) {
        match network::receive_from_local_socket_with_control(
            socket,
            output,
            flags & MSG_PEEK != 0,
            flags & MSG_OOB != 0,
            false,
        ) {
            Ok(result) => (result.length, None),
            Err(error) => return fail_io(error),
        }
    } else {
        match network::receive_from(socket, output) {
            Ok(result) => result,
            Err(error) => return fail_io(error),
        }
    };
    if !address.is_null() {
        if network::is_local_socket_fd(socket) {
            let peer_path = match network::local_socket_names(socket, true) {
                Ok(path) => path,
                Err(error) => return fail_io(error),
            };
            if let Err(error) = unsafe {
                write_local_sockaddr(address, address_length, &peer_path)
            } {
                return fail_io(error);
            }
        } else {
            let (peer_address, peer_port) =
                match source.or_else(|| network::endpoint(socket, true).ok()) {
                    Some(endpoint) => endpoint,
                    None => return fail_io(Error::NotConn),
                };
            if let Err(error) = unsafe {
                write_sockaddr(address, address_length, peer_address, peer_port)
            } {
                return fail_io(error);
            }
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
    if header.control_length as usize > MAX_CONTROL_LENGTH
        || (header.control_length != 0 && header.control.is_null())
    {
        return fail_io(if header.control.is_null() {
            Error::Fault
        } else {
            Error::TooBig
        });
    }
    if !network::is_local_socket_fd(socket)
        && (header.control_length != 0 || flags & (MSG_OOB | MSG_PEEK | MSG_CMSG_CLOEXEC) != 0)
    {
        return fail_io(Error::OpNotSupp);
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
    let (received, source, mut control_messages) = if network::is_local_socket_fd(socket) {
        match network::receive_from_local_socket_with_control(
            socket,
            &mut payload,
            flags & MSG_PEEK != 0,
            flags & MSG_OOB != 0,
            true,
        ) {
            Ok(result) => (result.length, None, result.control_messages),
            Err(error) => return fail_io(error),
        }
    } else {
        match network::receive_from(socket, &mut payload) {
            Ok((length, source)) => (length, source, Vec::new()),
            Err(error) => return fail_io(error),
        }
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
        if network::is_local_socket_fd(socket) {
            let peer_path = match network::local_socket_names(socket, true) {
                Ok(path) => path,
                Err(error) => {
                    if flags & MSG_PEEK == 0 {
                        close_control_messages(control_messages);
                    }
                    unsafe { ptr::write_unaligned(message.cast::<Msghdr>(), header) };
                    return fail_io(error);
                }
            };
            if let Err(error) = unsafe {
                write_local_sockaddr(header.name, &mut header.name_length, &peer_path)
            } {
                if flags & MSG_PEEK == 0 {
                    close_control_messages(control_messages);
                }
                unsafe { ptr::write_unaligned(message.cast::<Msghdr>(), header) };
                return fail_io(error);
            }
        } else {
            let peer = source.or_else(|| network::endpoint(socket, true).ok());
            let Some((address, port)) = peer else {
                if flags & MSG_PEEK == 0 {
                    close_control_messages(control_messages);
                }
                return fail_io(Error::NotConn);
            };
            if let Err(error) = unsafe {
                write_sockaddr(header.name, &mut header.name_length, address, port)
            } {
                if flags & MSG_PEEK == 0 {
                    close_control_messages(control_messages);
                }
                unsafe { ptr::write_unaligned(message.cast::<Msghdr>(), header) };
                return fail_io(error);
            }
        }
    }
    match write_control_messages(
        &mut header,
        core::mem::take(&mut control_messages),
        flags & MSG_PEEK != 0,
        flags & MSG_CMSG_CLOEXEC != 0,
    ) {
        Ok(true) => header.flags |= MSG_CTRUNC,
        Ok(false) => {}
        Err(error) => return fail_io(error),
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
    if flags & !(MSG_NOSIGNAL | MSG_OOB) != 0 || (buffer.is_null() && length != 0) {
        return fail_io(Error::Inval);
    }
    if flags & MSG_OOB != 0 && !network::is_local_socket_fd(socket) {
        return fail_io(Error::OpNotSupp);
    }
    let input = if length == 0 {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(buffer.cast::<u8>(), length) }
    };
    let result = if network::is_local_socket_fd(socket) {
        network::send_to_local_socket_with_control(
            socket,
            None,
            input,
            None,
            flags & MSG_OOB != 0,
        )
    } else {
        network::send(socket, input)
    };
    match result {
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
    if message.is_null() || flags & !(MSG_NOSIGNAL | MSG_OOB) != 0 {
        return fail_io(if message.is_null() { Error::Fault } else { Error::Inval });
    }
    let header = unsafe { ptr::read_unaligned(message.cast::<Msghdr>()) };
    if header.control_length as usize > MAX_CONTROL_LENGTH
        || (header.control_length != 0 && header.control.is_null())
    {
        return fail_io(if header.control.is_null() {
            Error::Fault
        } else {
            Error::TooBig
        });
    }
    if !network::is_local_socket_fd(socket) && header.control_length != 0 {
        return fail_io(Error::OpNotSupp);
    }
    if flags & MSG_OOB != 0 && !network::is_local_socket_fd(socket) {
        return fail_io(Error::OpNotSupp);
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
    let mut control_message = if network::is_local_socket_fd(socket) {
        match unsafe { parse_control_messages(header.control, header.control_length) } {
            Ok(control) => control,
            Err(error) => return fail_io(error),
        }
    } else {
        None
    };
    if network::is_local_socket_fd(socket) {
        let destination = if header.name.is_null() {
            None
        } else {
            let path = match unsafe { read_local_sockaddr(header.name, header.name_length) } {
                Ok(path) => path,
                Err(error) => {
                    if let Some(control) = control_message {
                        close_control_rights(control.rights);
                    }
                    return fail_io(error);
                }
            };
            match crate::kernel::runtime::runtime_libc::filesystem::normalize_local_socket_path(&path) {
                Ok(path) => Some(path),
                Err(error) => {
                    if let Some(control) = control_message {
                        close_control_rights(control.rights);
                    }
                    unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
                    return -1;
                }
            }
        };
        let mut control_rights = Vec::new();
        let rights_count = control_message
            .as_ref()
            .map_or(0, |control| control.rights.len());
        if control_rights.try_reserve_exact(rights_count).is_err() {
            if let Some(control) = control_message.take() {
                close_control_rights(control.rights);
            }
            return fail_io(Error::NoMem);
        }
        if let Some(control) = control_message.as_ref() {
            control_rights.extend_from_slice(&control.rights);
        }
        return match network::send_to_local_socket_with_control(
            socket,
            destination.as_deref(),
            &payload,
            control_message,
            flags & MSG_OOB != 0,
        ) {
            Ok(amount) => amount as isize,
            Err(error) => {
                close_control_rights(control_rights);
                fail_io(error)
            }
        };
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
    if flags & !(MSG_NOSIGNAL | MSG_OOB) != 0 || (buffer.is_null() && length != 0) {
        return fail_io(Error::Inval);
    }
    if flags & MSG_OOB != 0 && !network::is_local_socket_fd(socket) {
        return fail_io(Error::OpNotSupp);
    }
    if network::is_local_socket_fd(socket) {
        let destination = if address.is_null() {
            None
        } else {
            let path = match unsafe { read_local_sockaddr(address, address_length) } {
                Ok(path) => path,
                Err(error) => return fail_io(error),
            };
            match crate::kernel::runtime::runtime_libc::filesystem::normalize_local_socket_path(&path) {
                Ok(path) => Some(path),
                Err(error) => {
                    unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
                    return -1;
                }
            }
        };
        let input = if length == 0 {
            &[][..]
        } else {
            unsafe { core::slice::from_raw_parts(buffer.cast::<u8>(), length) }
        };
        return match network::send_to_local_socket_with_control(
            socket,
            destination.as_deref(),
            input,
            None,
            flags & MSG_OOB != 0,
        ) {
            Ok(amount) => amount as isize,
            Err(error) => fail_io(error),
        };
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
pub unsafe extern "C" fn sockatmark(socket: c_int) -> c_int {
    match network::socket_at_mark(socket) {
        Ok(at_mark) => i32::from(at_mark),
        Err(error) => fail(error),
    }
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
    domain: c_int,
    socket_type: c_int,
    protocol: c_int,
    sockets: *mut c_int,
) -> c_int {
    if sockets.is_null() {
        return fail(Error::Fault);
    }
    if domain != AF_UNIX {
        return fail(Error::AfNoSupport);
    }
    if socket_type & !(SOCK_NONBLOCK | SOCK_CLOEXEC | SOCK_CLOFORK | 0x0f) != 0 {
        return fail(Error::Inval);
    }
    let (first, second) = match network::create_local_socket_pair(domain, socket_type, protocol) {
        Ok(descriptors) => descriptors,
        Err(error) => return fail(error),
    };
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => {
            let _ = network::close(first);
            let _ = network::close(second);
            return fail(error);
        }
    };
    let descriptors = [first, second];
    let output = unsafe {
        core::slice::from_raw_parts(descriptors.as_ptr().cast::<u8>(), 2 * size_of::<c_int>())
    };
    if let Err(error) = process.address_space().write(sockets as usize, output) {
        let _ = network::close(first);
        let _ = network::close(second);
        return fail(error);
    }
    0
}