/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::error::Error;
use crate::kernel::net::tcp::connection::{TcpSocketHandle, TcpState};
use crate::kernel::net::stack::NetStack;
use crate::kernel::net::wire::Ipv4Address;

pub(crate) mod segment;
pub(crate) mod connection;

pub(crate) struct Tcp;

impl Tcp {
    pub(crate) fn connect(
        stack: &mut NetStack,
        local_address: Ipv4Address,
        address: Ipv4Address,
        remote_port: u16,
        local_port: u16,
        now_ns: u64,
    ) -> Result<TcpSocketHandle, Error> {
        stack.tcp_sockets.connect(
            local_address,
            address,
            local_port,
            remote_port,
            now_ns,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn listen(
        stack: &mut NetStack,
        local_address: Ipv4Address,
        local_port: u16,
        backlog: usize,
        now_ns: u64,
    ) -> Result<TcpSocketHandle, Error> {
        stack
            .tcp_sockets
            .listen(local_address, local_port, backlog, now_ns)
    }

    pub(crate) fn accept(
        stack: &mut NetStack,
        listener: TcpSocketHandle,
    ) -> Result<Option<TcpSocketHandle>, Error> {
        stack.tcp_sockets.accept(listener)
    }

    pub(crate) fn send(
        stack: &mut NetStack,
        handle: TcpSocketHandle,
        bytes: &[u8],
    ) -> Result<usize, Error> {
        stack.tcp_sockets.send(handle, bytes)
    }

    pub(crate) fn receive(
        stack: &mut NetStack,
        handle: TcpSocketHandle,
        output: &mut [u8],
    ) -> Result<usize, Error> {
        stack.tcp_sockets.receive(handle, output)
    }

    pub(crate) fn close(
        stack: &mut NetStack,
        handle: TcpSocketHandle,
        now_ns: u64,
    ) -> Result<(), Error> {
        stack.tcp_sockets.close(handle, now_ns)
    }

    pub(crate) fn shutdown_write(
        stack: &mut NetStack,
        handle: TcpSocketHandle,
    ) -> Result<(), Error> {
        stack.tcp_sockets.shutdown_write(handle)
    }

    pub(crate) fn remove(stack: &mut NetStack, handle: TcpSocketHandle) -> bool {
        stack.tcp_sockets.remove(handle)
    }

    pub(crate) fn state(stack: &NetStack, handle: TcpSocketHandle) -> Option<TcpState> {
        stack.tcp_sockets.state(handle)
    }

    pub(crate) fn readable(stack: &NetStack, handle: TcpSocketHandle) -> bool {
        stack.tcp_sockets.readable(handle)
    }

    pub(crate) fn writable(stack: &NetStack, handle: TcpSocketHandle) -> bool {
        stack.tcp_sockets.writable(handle)
    }

    pub(crate) fn local_endpoint(
        stack: &NetStack,
        handle: TcpSocketHandle,
    ) -> Option<(Ipv4Address, u16)> {
        stack.tcp_sockets.local_endpoint(handle)
    }

    pub(crate) fn peer_endpoint(
        stack: &NetStack,
        handle: TcpSocketHandle,
    ) -> Option<(Ipv4Address, u16)> {
        stack.tcp_sockets.peer_endpoint(handle)
    }
}
