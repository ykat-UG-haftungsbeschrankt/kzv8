/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::c_void;
use core::slice;

use crate::kernel::arch::traits::{CpuArchitecture as _, TimerArchitecture as _};
use crate::kernel::block::service::BlockClient;
use crate::kernel::error::Error;
use crate::kernel::event::r#loop::{
    Completion, EventFlags, EventLoop, EventSourceId, EventSourceToken, RawEvent,
};
use crate::kernel::event::timers::TimerId;
use crate::kernel::net::stack::NetStack;
use smoltcp::iface::SocketHandle;
use smoltcp::socket::tcp::{Socket as TcpSocket, State as TcpState};
use smoltcp::wire::Ipv4Address;

pub(crate) struct TcpContext {
    event_loop: *mut EventLoop<16>,
    network_stack: *mut NetStack,
    network_registration: Option<u64>,
    socket_handle: Option<SocketHandle>,
}

impl TcpContext {
    pub(crate) fn new(
        event_loop: &mut EventLoop<16>,
        network_stack: Option<&mut NetStack>,
        network_registration: Option<EventSourceToken>,
    ) -> Self {
        Self {
            event_loop,
            network_stack: network_stack.map_or(core::ptr::null_mut(), |stack| stack),
            network_registration: network_registration.map(EventSourceToken::raw),
            socket_handle: None,
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_tcp_available(context: *mut c_void) -> i32 {
    if context.is_null() {
        return 0;
    }
    (!unsafe { &*context.cast::<TcpContext>() }
        .network_stack
        .is_null()) as i32
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_console_write(data: *const u8, length: usize) -> i32 {
    if length > 512 || (data.is_null() && length != 0) {
        return -1;
    }
    let bytes = if length == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(data, length)
    };
    if crate::kernel::driver::virtio::console::Console::write(bytes) {
        length as i32
    } else {
        -1
    }
}

#[no_mangle]
pub extern "C" fn kz_host_time_ns() -> u64 {
    match crate::kernel::arch::TimerArchitecture::now_ns() {
        Some(now_ns) if now_ns != 0 => now_ns,
        _ => crate::kernel::arch::TimerArchitecture::fallback_time_ns(),
    }
}

#[no_mangle]
pub extern "C" fn kz_host_wall_time_ms() -> i64 {
    0
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_poll(
    context: *mut c_void,
    output: *mut RawEvent,
) -> i32 {
    if context.is_null() || output.is_null() {
        return 0;
    }
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    let now_ns =
        crate::kernel::arch::TimerArchitecture::now_ns().unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns);
    let Some(event) = events.poll_raw(now_ns) else {
        return 0;
    };
    unsafe { output.write(event) };
    1
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_schedule_timer(
    context: *mut c_void,
    delay_ns: u64,
    timer_id: *mut u64,
) -> i32 {
    if context.is_null() || timer_id.is_null() {
        return 0;
    }
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    let now_ns =
        crate::kernel::arch::TimerArchitecture::now_ns().unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns);
    let Some(deadline_ns) = now_ns.checked_add(delay_ns) else {
        return 0;
    };
    let Some(timer) = events.schedule_timer(deadline_ns) else {
        return 0;
    };
    unsafe { timer_id.write(timer.raw()) };
    1
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_cancel_timer(
    context: *mut c_void,
    timer_id: u64,
) -> i32 {
    if context.is_null() {
        return 0;
    }
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    events.cancel_timer(TimerId::from_raw(timer_id)) as i32
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_register(
    context: *mut c_void,
    source_id: u64,
    interest: u32,
    data: u64,
    token: *mut u64,
) -> i32 {
    if context.is_null() || token.is_null() {
        return 0;
    }
    let Some(interest) = EventFlags::from_bits(interest) else {
        return 0;
    };
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    let Ok(source_token) = events.register(EventSourceId::new(source_id), interest, data) else {
        return 0;
    };
    unsafe { token.write(source_token.raw()) };
    1
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_modify(
    context: *mut c_void,
    token: u64,
    interest: u32,
    data: u64,
) -> i32 {
    if context.is_null() {
        return 0;
    }
    let Some(interest) = EventFlags::from_bits(interest) else {
        return 0;
    };
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    events
        .modify(EventSourceToken::from_raw(token), interest, data)
        .is_ok() as i32
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_set_readiness(
    context: *mut c_void,
    token: u64,
    readiness: u32,
) -> i32 {
    if context.is_null() {
        return 0;
    }
    let Some(readiness) = EventFlags::from_bits(readiness) else {
        return 0;
    };
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    events
        .set_readiness(EventSourceToken::from_raw(token), readiness)
        .is_ok() as i32
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_deregister(context: *mut c_void, token: u64) -> i32 {
    if context.is_null() {
        return 0;
    }
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    events.deregister(EventSourceToken::from_raw(token)).is_ok() as i32
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_complete(
    context: *mut c_void,
    id: u64,
    status: i32,
) -> i32 {
    if context.is_null() {
        return 0;
    }
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    events.complete(Completion { id, status }).is_ok() as i32
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_next_deadline(
    context: *mut c_void,
    deadline_ns: *mut u64,
) -> i32 {
    if context.is_null() || deadline_ns.is_null() {
        return 0;
    }
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    let Some(deadline) = events.next_deadline() else {
        return 0;
    };
    unsafe { deadline_ns.write(deadline) };
    1
}

const MAX_BLOCK_TRANSFER_SIZE: usize = 64 * 1024;
const MAX_TCP_TRANSFER_SIZE: usize = 4096;
const TCP_TIMEOUT_NS: u64 = 5_000_000_000;
const TCP_SPINS: usize = 10_000_000;

#[no_mangle]
pub unsafe extern "C" fn kz_host_block_capacity(device: *mut c_void) -> u64 {
    if device.is_null() {
        return 0;
    }
    unsafe { &*device.cast::<BlockClient>() }.capacity_sectors()
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_block_read(
    device: *mut c_void,
    sector: u64,
    output: *mut u8,
    length: usize,
) -> i32 {
    if device.is_null() || output.is_null() || length == 0 || length > MAX_BLOCK_TRANSFER_SIZE {
        return -1;
    }
    let output = unsafe { slice::from_raw_parts_mut(output, length) };
    match unsafe { &*device.cast::<BlockClient>() }.read(sector, output) {
        Ok(()) => length as i32,
        Err(error) => block_error_code(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_block_write(
    device: *mut c_void,
    sector: u64,
    input: *const u8,
    length: usize,
) -> i32 {
    if device.is_null() || input.is_null() || length == 0 || length > MAX_BLOCK_TRANSFER_SIZE {
        return -1;
    }
    let input = unsafe { slice::from_raw_parts(input, length) };
    match unsafe { &*device.cast::<BlockClient>() }.write(sector, input) {
        Ok(()) => length as i32,
        Err(error) => block_error_code(error),
    }
}

fn block_error_code(error: Error) -> i32 {
    match error {
        Error::Inval | Error::Fault | Error::MsgSize => -1,
        Error::NoMem | Error::Busy | Error::Mfile => -2,
        Error::TimedOut => -4,
        _ => -3,
    }
}

fn host_time_ns() -> u64 {
    crate::kernel::arch::TimerArchitecture::now_ns().unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns)
}

#[no_mangle]
pub extern "C" fn kz_host_wait_for_interrupt() {
    crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
}

fn poll_tcp_context(context: &mut TcpContext) -> bool {
    if context.event_loop.is_null() || context.network_stack.is_null() {
        return false;
    }
    let source = EventSourceToken::from_raw(context.network_registration.unwrap_or(0));
    unsafe {
        (&mut *context.network_stack).poll(host_time_ns(), &mut *context.event_loop, source);
    }
    true
}

fn tcp_state(context: &mut TcpContext, handle: SocketHandle) -> Option<TcpState> {
    if context.network_stack.is_null() {
        return None;
    }
    Some(
        unsafe { &mut *context.network_stack }
            .sockets
            .get::<TcpSocket>(handle)
            .state(),
    )
}

fn remove_tcp_socket(context: &mut TcpContext, handle: SocketHandle) {
    if !context.network_stack.is_null() {
        let _socket = unsafe { &mut *context.network_stack }
            .sockets
            .remove(handle);
    }
    context.socket_handle = None;
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_tcp_connect(
    context: *mut c_void,
    address: *const u8,
    remote_port: u32,
) -> i32 {
    if context.is_null() || address.is_null() || remote_port == 0 || remote_port > u16::MAX as u32 {
        return -1;
    }
    let context = unsafe { &mut *context.cast::<TcpContext>() };
    if context.network_stack.is_null() {
        return -2;
    }
    if context.socket_handle.is_some() {
        return -3;
    }
    let address = unsafe { slice::from_raw_parts(address, 4) };
    let address = Ipv4Address::new(address[0], address[1], address[2], address[3]);
    let handle = match crate::kernel::net::tcp::Tcp::connect(
        unsafe { &mut *context.network_stack },
        address,
        remote_port as u16,
        49_152,
    ) {
        Ok(handle) => handle,
        Err(_) => return -4,
    };
    context.socket_handle = Some(handle);
    let started = host_time_ns();
    let mut failure = -6;
    for _ in 0..TCP_SPINS {
        if !poll_tcp_context(context) {
            failure = -7;
            break;
        }
        match tcp_state(context, handle) {
            Some(TcpState::Established) => return 1,
            Some(TcpState::Closed) | None => {
                failure = -5;
                break;
            }
            _ => {}
        }
        if host_time_ns().saturating_sub(started) >= TCP_TIMEOUT_NS {
            break;
        }
        crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
    }
    remove_tcp_socket(context, handle);
    failure
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_tcp_send(
    context: *mut c_void,
    input: *const u8,
    length: usize,
) -> i32 {
    if context.is_null() || (input.is_null() && length != 0) || length > MAX_TCP_TRANSFER_SIZE {
        return -1;
    }
    let context = unsafe { &mut *context.cast::<TcpContext>() };
    let Some(handle) = context.socket_handle else {
        return -1;
    };
    let input = unsafe { slice::from_raw_parts(input, length) };
    let mut written = 0;
    let started = host_time_ns();
    for _ in 0..TCP_SPINS {
        if !poll_tcp_context(context) {
            return -1;
        }
        if written < length && tcp_state(context, handle) == Some(TcpState::Established) {
            let count = crate::kernel::net::tcp::Tcp::send(
                unsafe { &mut *context.network_stack },
                handle,
                &input[written..],
            )
            .unwrap_or(0);
            written += count;
        }
        if written == length {
            return length as i32;
        }
        if host_time_ns().saturating_sub(started) >= TCP_TIMEOUT_NS {
            break;
        }
        crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_tcp_receive(
    context: *mut c_void,
    output: *mut u8,
    length: usize,
) -> i32 {
    if context.is_null() || output.is_null() || length == 0 || length > MAX_TCP_TRANSFER_SIZE {
        return -1;
    }
    let context = unsafe { &mut *context.cast::<TcpContext>() };
    let Some(handle) = context.socket_handle else {
        return -1;
    };
    let output = unsafe { slice::from_raw_parts_mut(output, length) };
    let mut received = 0;
    let started = host_time_ns();
    for _ in 0..TCP_SPINS {
        if !poll_tcp_context(context) {
            return -1;
        }
        match tcp_state(context, handle) {
            Some(TcpState::Closed) | None => break,
            _ => {}
        }
        let count = crate::kernel::net::tcp::Tcp::receive(
            unsafe { &mut *context.network_stack },
            handle,
            &mut output[received..],
        )
        .unwrap_or(0);
        received += count;
        if received == length {
            return received as i32;
        }
        if host_time_ns().saturating_sub(started) >= TCP_TIMEOUT_NS {
            break;
        }
        crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
    }
    received as i32
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_tcp_close(context: *mut c_void) -> i32 {
    if context.is_null() {
        return 0;
    }
    let context = unsafe { &mut *context.cast::<TcpContext>() };
    let Some(handle) = context.socket_handle else {
        return 0;
    };
    if !context.network_stack.is_null() {
        unsafe { &mut *context.network_stack }
            .sockets
            .get_mut::<TcpSocket>(handle)
            .close();
        poll_tcp_context(context);
    }
    remove_tcp_socket(context, handle);
    1
}
