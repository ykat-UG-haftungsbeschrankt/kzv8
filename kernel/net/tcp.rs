/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec;
use smoltcp::iface::SocketHandle;
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer};
use smoltcp::wire::{IpAddress, IpEndpoint, Ipv4Address};

use crate::kernel::error::Error;
use crate::kernel::net::stack::NetStack;

pub(crate) struct Tcp;

impl Tcp {
    const DEFAULT_RX_CAPACITY: usize = 4096;
    const DEFAULT_TX_CAPACITY: usize = 4096;

    pub(crate) fn open_socket(
        rx_capacity: usize,
        tx_capacity: usize,
    ) -> Result<TcpSocket<'static>, Error> {
        if rx_capacity == 0 || tx_capacity == 0 {
            return Err(Error::Inval);
        }
        Ok(TcpSocket::new(
            SocketBuffer::new(vec![0; rx_capacity]),
            SocketBuffer::new(vec![0; tx_capacity]),
        ))
    }

    pub(crate) fn connect(
        stack: &mut NetStack,
        address: Ipv4Address,
        remote_port: u16,
        local_port: u16,
    ) -> Result<SocketHandle, Error> {
        let mut socket = Self::open_socket(Self::DEFAULT_RX_CAPACITY, Self::DEFAULT_TX_CAPACITY)?;
        socket
            .connect(
                stack.interface.context(),
                IpEndpoint::new(IpAddress::Ipv4(address), remote_port),
                local_port,
            )
            .map_err(|_| Error::Io)?;
        Ok(stack.sockets.add(socket))
    }

    pub(crate) fn listen(stack: &mut NetStack, local_port: u16) -> Result<SocketHandle, Error> {
        let mut socket = Self::open_socket(Self::DEFAULT_RX_CAPACITY, Self::DEFAULT_TX_CAPACITY)?;
        socket.listen(local_port).map_err(|_| Error::Io)?;
        Ok(stack.sockets.add(socket))
    }

    pub(crate) fn send(
        stack: &mut NetStack,
        handle: SocketHandle,
        bytes: &[u8],
    ) -> Result<usize, Error> {
        stack
            .sockets
            .get_mut::<TcpSocket>(handle)
            .send_slice(bytes)
            .map_err(|_| Error::Io)
    }

    pub(crate) fn receive(
        stack: &mut NetStack,
        handle: SocketHandle,
        output: &mut [u8],
    ) -> Result<usize, Error> {
        stack
            .sockets
            .get_mut::<TcpSocket>(handle)
            .recv_slice(output)
            .map_err(|_| Error::Io)
    }
}
