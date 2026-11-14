/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;
use core::ffi::{c_char, c_void};
use core::ffi::CStr;
use core::slice;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::kernel::arch::traits::{CpuArchitecture as _, TimerArchitecture as _};
use crate::kernel::block::service::BlockClient;
use crate::kernel::block::BlockClientApi as _;
use crate::kernel::error::Error;
use crate::kernel::event::{
    Completion, Event, EventFlags, EventFlagsApi, EventLoop, EventLoopApi as _, EventSourceId,
    EventSourceIdApi, EventSourceToken, EventSourceTokenApi,
};
use crate::kernel::process::KernelObjectRef;
use crate::kernel::net::tcp::connection::{TcpSocketHandle, TcpState};
use crate::kernel::net::stack::NetStack;
use crate::kernel::net::tcp::Tcp;
use crate::kernel::net::udp::UdpSocketHandle;
use crate::kernel::net::wire::Ipv4Address;
use crate::kernel::vfs::plugin::tun::TunDevice;

static ACTIVE_TCP_CONTEXT: AtomicPtr<TcpContext> = AtomicPtr::new(core::ptr::null_mut());

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RawEvent {
    kind: u32,
    flags: u32,
    id: u64,
    status: i32,
    reserved: u32,
    data: u64,
}

pub(crate) struct TcpContext {
    event_loop: *mut EventLoop<16>,
    network_stack: *mut NetStack,
    network_registration: Option<u64>,
    socket_handle: Option<TcpSocketHandle>,
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

    pub(crate) fn install_active(&mut self) {
        ACTIVE_TCP_CONTEXT.store(self, Ordering::Release);
    }

    pub(crate) fn clear_active(&mut self) {
        let _ = ACTIVE_TCP_CONTEXT.compare_exchange(
            self,
            core::ptr::null_mut(),
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    fn schedule_timer_at(&mut self, deadline_ns: u64) -> Option<u64> {
        let events = unsafe { &mut *self.event_loop };
        let timer = events.schedule_timer(deadline_ns)?;
        Some(KernelObjectRef::from_arc(&timer).raw())
    }

    fn cancel_timer(&mut self, timer_id: u64) -> bool {
        let events = unsafe { &mut *self.event_loop };
        let timer_reference = KernelObjectRef::from_raw(timer_id);
        let Some(timer) = events.timer_by_address(timer_reference.raw()) else {
            return false;
        };
        events.cancel_timer(&timer)
    }

    pub(crate) fn poll_network(&mut self) -> bool {
        poll_tcp_context(self)
    }

    pub(crate) fn local_ipv4(&self) -> Option<Ipv4Address> {
        if self.network_stack.is_null() {
            None
        } else {
            Some(unsafe { &*self.network_stack }.ipv4_address())
        }
    }

    pub(crate) fn local_ipv4_prefix_length(&self) -> Option<u8> {
        if self.network_stack.is_null() {
            None
        } else {
            Some(unsafe { &*self.network_stack }.ipv4_prefix_length())
        }
    }

    pub(crate) fn set_host_ipv4_configuration(
        &mut self,
        address: Ipv4Address,
        prefix_length: u8,
    ) -> Result<(), Error> {
        self.network_stack_mut()?
            .set_ipv4_configuration(address, prefix_length)
    }

    pub(crate) fn set_host_ipv4_ttl(&mut self, ttl: u8) -> Result<(), Error> {
        self.network_stack_mut()?.set_default_ttl(ttl)
    }

    pub(crate) fn set_network_receive_budget(&mut self, budget: usize) -> Result<(), Error> {
        self.network_stack_mut()?.set_receive_budget(budget)
    }

    pub(crate) fn is_local_ipv4_address(&self, address: Ipv4Address) -> bool {
        !self.network_stack.is_null()
            && unsafe { &*self.network_stack }.is_local_ipv4_address(address)
    }

    pub(crate) fn is_ipv4_broadcast_address(&self, address: Ipv4Address) -> bool {
        !self.network_stack.is_null()
            && unsafe { &*self.network_stack }.is_broadcast_address(address)
    }

    pub(crate) fn ethernet_mac_address(&self) -> Option<[u8; 6]> {
        if self.network_stack.is_null() {
            None
        } else {
            Some(unsafe { &*self.network_stack }.ethernet_mac_address())
        }
    }

    pub(crate) fn ethernet_mtu(&self) -> Option<usize> {
        if self.network_stack.is_null() {
            None
        } else {
            Some(unsafe { &*self.network_stack }.ethernet_mtu())
        }
    }

    pub(crate) fn set_ipv4_forwarding(&mut self, enabled: bool) -> Result<(), Error> {
        self.network_stack_mut()?.set_ipv4_forwarding(enabled);
        Ok(())
    }

    pub(crate) fn add_ipv4_route(
        &mut self,
        network: Ipv4Address,
        prefix_length: u8,
        gateway: Option<Ipv4Address>,
    ) -> Result<(), Error> {
        self.network_stack_mut()?
            .add_ipv4_route(network, prefix_length, gateway)
    }

    pub(crate) fn add_ipv4_tun_route(
        &mut self,
        network: Ipv4Address,
        prefix_length: u8,
        device: Arc<TunDevice>,
    ) -> Result<(), Error> {
        self.network_stack_mut()?
            .add_ipv4_tun_route(network, prefix_length, device)
    }

    pub(crate) fn delete_ipv4_route(
        &mut self,
        network: Ipv4Address,
        prefix_length: u8,
    ) -> Result<(), Error> {
        self.network_stack_mut()?
            .remove_ipv4_route(network, prefix_length)
    }

    pub(crate) fn tcp_connect(
        &mut self,
        local_address: Ipv4Address,
        remote_address: Ipv4Address,
        remote_port: u16,
        local_port: u16,
    ) -> Result<TcpSocketHandle, Error> {
        Tcp::connect(
            self.network_stack_mut()?,
            local_address,
            remote_address,
            remote_port,
            local_port,
            host_time_ns(),
        )
    }

    pub(crate) fn tcp_listen(
        &mut self,
        local_address: Ipv4Address,
        local_port: u16,
        backlog: usize,
    ) -> Result<TcpSocketHandle, Error> {
        Tcp::listen(
            self.network_stack_mut()?,
            local_address,
            local_port,
            backlog,
            host_time_ns(),
        )
    }

    pub(crate) fn tcp_accept(
        &mut self,
        listener: TcpSocketHandle,
    ) -> Result<Option<TcpSocketHandle>, Error> {
        Tcp::accept(self.network_stack_mut()?, listener)
    }

    pub(crate) fn tcp_send(
        &mut self,
        handle: TcpSocketHandle,
        bytes: &[u8],
    ) -> Result<usize, Error> {
        Tcp::send(self.network_stack_mut()?, handle, bytes)
    }

    pub(crate) fn tcp_receive(
        &mut self,
        handle: TcpSocketHandle,
        output: &mut [u8],
    ) -> Result<usize, Error> {
        Tcp::receive(self.network_stack_mut()?, handle, output)
    }

    pub(crate) fn tcp_close(&mut self, handle: TcpSocketHandle) -> Result<(), Error> {
        Tcp::close(self.network_stack_mut()?, handle, host_time_ns())
    }

    pub(crate) fn tcp_shutdown_write(
        &mut self,
        handle: TcpSocketHandle,
    ) -> Result<(), Error> {
        Tcp::shutdown_write(self.network_stack_mut()?, handle)
    }

    pub(crate) fn tcp_remove(&mut self, handle: TcpSocketHandle) -> bool {
        if self.network_stack.is_null() {
            return false;
        }
        Tcp::remove(unsafe { &mut *self.network_stack }, handle)
    }

    pub(crate) fn tcp_state(&self, handle: TcpSocketHandle) -> Option<TcpState> {
        if self.network_stack.is_null() {
            None
        } else {
            Tcp::state(unsafe { &*self.network_stack }, handle)
        }
    }

    pub(crate) fn tcp_readable(&self, handle: TcpSocketHandle) -> bool {
        !self.network_stack.is_null()
            && Tcp::readable(unsafe { &*self.network_stack }, handle)
    }

    pub(crate) fn tcp_writable(&self, handle: TcpSocketHandle) -> bool {
        !self.network_stack.is_null()
            && Tcp::writable(unsafe { &*self.network_stack }, handle)
    }

    pub(crate) fn tcp_local_endpoint(
        &self,
        handle: TcpSocketHandle,
    ) -> Option<(Ipv4Address, u16)> {
        if self.network_stack.is_null() {
            None
        } else {
            Tcp::local_endpoint(unsafe { &*self.network_stack }, handle)
        }
    }

    pub(crate) fn tcp_peer_endpoint(
        &self,
        handle: TcpSocketHandle,
    ) -> Option<(Ipv4Address, u16)> {
        if self.network_stack.is_null() {
            None
        } else {
            Tcp::peer_endpoint(unsafe { &*self.network_stack }, handle)
        }
    }

    pub(crate) fn udp_create(&mut self) -> Result<UdpSocketHandle, Error> {
        self.network_stack_mut()?.udp_sockets.create()
    }

    pub(crate) fn udp_bind(
        &mut self,
        handle: UdpSocketHandle,
        address: Ipv4Address,
        port: u16,
    ) -> Result<(), Error> {
        self.network_stack_mut()?.udp_sockets.bind(handle, address, port)
    }

    pub(crate) fn udp_connect(
        &mut self,
        handle: UdpSocketHandle,
        address: Ipv4Address,
        port: u16,
    ) -> Result<(), Error> {
        self.network_stack_mut()?
            .udp_sockets
            .connect(handle, address, port)
    }

    pub(crate) fn udp_send(
        &mut self,
        handle: UdpSocketHandle,
        address: Ipv4Address,
        port: u16,
        bytes: &[u8],
    ) -> Result<usize, Error> {
        self.network_stack_mut()?
            .send_udp(handle, address, port, bytes, host_time_ns())
    }

    pub(crate) fn udp_receive(
        &mut self,
        handle: UdpSocketHandle,
        output: &mut [u8],
    ) -> Result<Option<(usize, Ipv4Address, u16)>, Error> {
        self.network_stack_mut()?.udp_sockets.receive(handle, output)
    }

    pub(crate) fn udp_close(&mut self, handle: UdpSocketHandle) -> bool {
        if self.network_stack.is_null() {
            return false;
        }
        unsafe { &mut *self.network_stack }
            .udp_sockets
            .close(handle)
    }

    pub(crate) fn udp_endpoint(
        &self,
        handle: UdpSocketHandle,
    ) -> Option<(Ipv4Address, u16, Option<(Ipv4Address, u16)>)> {
        if self.network_stack.is_null() {
            None
        } else {
            unsafe { &*self.network_stack }
                .udp_sockets
                .endpoint(handle)
                .ok()
        }
    }

    pub(crate) fn udp_readable(&self, handle: UdpSocketHandle) -> bool {
        !self.network_stack.is_null()
            && unsafe { &*self.network_stack }
                .udp_sockets
                .readable(handle)
    }

    fn network_stack_mut(&mut self) -> Result<&mut NetStack, Error> {
        if self.network_stack.is_null() {
            return Err(Error::NoDev);
        }
        // SAFETY: TcpContext is the sole runtime-facing owner of this mutable stack reference.
        Ok(unsafe { &mut *self.network_stack })
    }
}

pub(crate) fn with_active_tcp_context<R>(
    operation: impl FnOnce(&mut TcpContext) -> R,
) -> Option<R> {
    let context = ACTIVE_TCP_CONTEXT.load(Ordering::Acquire);
    if context.is_null() {
        return None;
    }
    // SAFETY: runtime::run installs this stack-local context for the duration of the
    // single active runtime call; the POSIX bridge is invoked synchronously on that thread.
    Some(operation(unsafe { &mut *context }))
}

pub(crate) fn schedule_active_timer_at(deadline_ns: u64) -> Option<u64> {
    with_active_tcp_context(|context| context.schedule_timer_at(deadline_ns)).flatten()
}

pub(crate) fn cancel_active_timer(timer_id: u64) -> bool {
    with_active_tcp_context(|context| context.cancel_timer(timer_id)).unwrap_or(false)
}

fn set_network_errno(error: Error) {
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(-(error as i32)) };
}

fn network_config_result(result: Option<Result<(), Error>>) -> i32 {
    match result {
        Some(Ok(())) => 0,
        Some(Err(error)) => {
            set_network_errno(error);
            -1
        }
        None => {
            set_network_errno(Error::NoDev);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn kz_host_network_set_ipv4_forwarding(enabled: i32) -> i32 {
    if enabled != 0 && enabled != 1 {
        set_network_errno(Error::Inval);
        return -1;
    }
    network_config_result(with_active_tcp_context(|context| {
        context.set_ipv4_forwarding(enabled != 0)
    }))
}

#[no_mangle]
pub extern "C" fn kz_host_network_set_ipv4_ttl(ttl: u32) -> i32 {
    let Ok(ttl) = u8::try_from(ttl) else {
        set_network_errno(Error::Inval);
        return -1;
    };
    network_config_result(with_active_tcp_context(|context| {
        context.set_host_ipv4_ttl(ttl)
    }))
}

#[no_mangle]
pub extern "C" fn kz_host_network_set_receive_budget(budget: u32) -> i32 {
    let Ok(budget) = usize::try_from(budget) else {
        set_network_errno(Error::Inval);
        return -1;
    };
    network_config_result(with_active_tcp_context(|context| {
        context.set_network_receive_budget(budget)
    }))
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_network_add_ipv4_route(
    network: *const u8,
    prefix_length: u32,
    gateway: *const u8,
) -> i32 {
    if network.is_null() {
        set_network_errno(Error::Fault);
        return -1;
    }
    let Ok(prefix_length) = u8::try_from(prefix_length) else {
        set_network_errno(Error::Inval);
        return -1;
    };
    if prefix_length > 32 {
        set_network_errno(Error::Inval);
        return -1;
    }
    let network_bytes = unsafe { slice::from_raw_parts(network, 4) };
    let network_address = Ipv4Address([
        network_bytes[0],
        network_bytes[1],
        network_bytes[2],
        network_bytes[3],
    ]);
    let gateway_address = if gateway.is_null() {
        None
    } else {
        let bytes = unsafe { slice::from_raw_parts(gateway, 4) };
        Some(Ipv4Address([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    network_config_result(with_active_tcp_context(|context| {
        context.add_ipv4_route(network_address, prefix_length, gateway_address)
    }))
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_network_delete_ipv4_route(
    network: *const u8,
    prefix_length: u32,
) -> i32 {
    if network.is_null() {
        set_network_errno(Error::Fault);
        return -1;
    }
    let Ok(prefix_length) = u8::try_from(prefix_length) else {
        set_network_errno(Error::Inval);
        return -1;
    };
    if prefix_length > 32 {
        set_network_errno(Error::Inval);
        return -1;
    }
    let bytes = unsafe { slice::from_raw_parts(network, 4) };
    let network_address = Ipv4Address([bytes[0], bytes[1], bytes[2], bytes[3]]);
    network_config_result(with_active_tcp_context(|context| {
        context.delete_ipv4_route(network_address, prefix_length)
    }))
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_network_add_ipv4_tun_route(
    network: *const u8,
    prefix_length: u32,
    interface_name: *const c_char,
) -> i32 {
    if network.is_null() || interface_name.is_null() {
        set_network_errno(Error::Fault);
        return -1;
    }
    let Ok(prefix_length) = u8::try_from(prefix_length) else {
        set_network_errno(Error::Inval);
        return -1;
    };
    if prefix_length > 32 {
        set_network_errno(Error::Inval);
        return -1;
    }
    let name = unsafe { CStr::from_ptr(interface_name) }.to_bytes();
    let device = match crate::kernel::runtime::runtime_libc::network::tun_device_by_name(name) {
        Ok(device) => device,
        Err(error) => {
            set_network_errno(error);
            return -1;
        }
    };
    let bytes = unsafe { slice::from_raw_parts(network, 4) };
    let network_address = Ipv4Address([bytes[0], bytes[1], bytes[2], bytes[3]]);
    network_config_result(with_active_tcp_context(|context| {
        context.add_ipv4_tun_route(network_address, prefix_length, device)
    }))
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
    match crate::kernel::tty::write_console(bytes, false) {
        Ok(written) if written == length => length as i32,
        Ok(written) => written as i32,
        Err(_) => -1,
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
    let _network_progress = with_active_tcp_context(TcpContext::poll_network);
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    let now_ns =
        crate::kernel::arch::TimerArchitecture::now_ns().unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns);
    let Some(event) = events.wait(now_ns) else {
        return 0;
    };
    let event = match event {
        Event::Timer(timer) => {
            let timer_id = KernelObjectRef::from_arc(&timer).raw();
            crate::kernel::runtime::runtime_posix::time::dispatch_posix_timer_event(
                events, timer_id, now_ns,
            );
            RawEvent {
                kind: 1,
                id: timer_id,
                ..RawEvent::default()
            }
        }
        Event::Completion(completion) => RawEvent {
            kind: 2,
            id: completion.id,
            status: completion.status,
            ..RawEvent::default()
        },
        Event::Readiness {
            source,
            flags,
            data,
        } => RawEvent {
            kind: 3,
            flags: u32::from(flags.bits()),
            id: source.raw(),
            data,
            ..RawEvent::default()
        },
        Event::DeviceInterrupt { slot } => RawEvent {
            kind: 4,
            id: u64::from(slot),
            ..RawEvent::default()
        },
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
    unsafe { timer_id.write(KernelObjectRef::from_arc(&timer).raw()) };
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
    let timer_reference = KernelObjectRef::from_raw(timer_id);
    let Some(timer) = events.timer_by_address(timer_reference.raw()) else {
        return 0;
    };
    events.cancel_timer(&timer) as i32
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
    let Ok(source_token) = events.register(
        <EventSourceId as EventSourceIdApi>::new(source_id),
        interest,
        data,
    ) else {
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

fn tcp_state(context: &mut TcpContext, handle: TcpSocketHandle) -> Option<TcpState> {
    if context.network_stack.is_null() {
        return None;
    }
    Tcp::state(unsafe { &*context.network_stack }, handle)
}

fn remove_tcp_socket(context: &mut TcpContext, handle: TcpSocketHandle) {
    if !context.network_stack.is_null() {
        let _removed = Tcp::remove(unsafe { &mut *context.network_stack }, handle);
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
    let address = Ipv4Address([address[0], address[1], address[2], address[3]]);
    let local_address = unsafe { &*context.network_stack }.ipv4_address();
    let handle = match crate::kernel::net::tcp::Tcp::connect(
        unsafe { &mut *context.network_stack },
        local_address,
        address,
        remote_port as u16,
        49_152,
        host_time_ns(),
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
        let _closed = Tcp::close(
            unsafe { &mut *context.network_stack },
            handle,
            host_time_ns(),
        );
        poll_tcp_context(context);
    }
    context.socket_handle = None;
    1
}
