/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ops::Range;

use crate::kernel::error::Error;
use crate::kernel::event::{EventFlags, EventLoop, EventSourceToken};
use crate::kernel::scheduler::thread::vm::UserAddressSpace;

mod ipc;

use ipc::{EndpointCore, Message, ReplySlot, SharedMemoryCore};

pub(crate) use ipc::{process_capability_smoke, shared_mapping_smoke, smoke};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CapabilityRights(u8);

pub(crate) trait CapabilityRightsApi: Sized {
    const NONE: Self;
    const SEND: Self;
    const RECEIVE: Self;
    const READ: Self;
    const WRITE: Self;
    const TRANSFER: Self;
    const CLOSE: Self;

    fn union(self, other: Self) -> Self;
    fn contains(self, rights: Self) -> bool;
}

pub(crate) struct MessageBody {
    opcode: u32,
    //@FIXME: INDEXTYPE(payload); dynamic byte array with O(1) indexing and amortized O(1) append; keep as a contiguous message buffer.
    payload: Vec<u8>,
    //@FIXME: INDEXTYPE(capabilities); ordered indexed list; Vec::remove(index) shifts the suffix in O(n). A sqrt-decomposed BlockVec gives O(sqrt n) removal but requires replacing the contiguous capabilities() slice API; swap-remove is O(1) only if reordering is acceptable.
    capabilities: Vec<Capability>,
}

pub(crate) trait MessageBodyApi: Sized {
    fn new(opcode: u32, payload: &[u8]) -> Result<Self, Error>;
    fn opcode(&self) -> u32;
    fn payload(&self) -> &[u8];
    fn capabilities(&self) -> &[Capability];
    fn take_capability(&mut self, index: usize) -> Option<Capability>;
    fn into_parts(self) -> (u32, Vec<u8>, Vec<Capability>);
    fn push_capability(&mut self, capability: Capability) -> Result<(), Error>;
}

pub(crate) enum Capability {
    Endpoint(Endpoint),
    SharedMemory(SharedMemoryGrant),
}

pub(crate) trait CapabilityApi: Sized {
    fn endpoint(&self) -> Option<&Endpoint>;
    fn shared_memory(&self) -> Option<&SharedMemoryGrant>;
    fn duplicate(&self) -> Result<Self, Error>;
    fn restrict(&self, rights: CapabilityRights) -> Result<Self, Error>;
    fn prepare_transfer(&self, rights: CapabilityRights) -> Result<Self, Error>;
}

pub(crate) struct ReceivedMessage {
    body: MessageBody,
    reply: Option<ReplyCapability>,
}

pub(crate) trait ReceivedMessageApi: Sized {
    fn body(&self) -> &MessageBody;
    fn into_parts(self) -> (MessageBody, Option<ReplyCapability>);
}

#[derive(Clone)]
pub(crate) struct Endpoint {
    core: Arc<EndpointCore>,
    rights: CapabilityRights,
}

pub(crate) trait EndpointApi: Sized {
    fn channel(capacity: usize) -> Result<(Self, Self), Error>;
    fn restrict(&self, rights: CapabilityRights) -> Result<Self, Error>;
    fn register<const N: usize>(
        &self,
        events: &mut EventLoop<N>,
        interest: EventFlags,
        data: u64,
    ) -> Result<EventSourceToken, Error>;
    fn try_send(&self, body: MessageBody) -> Result<(), Error>;
    fn send(&self, body: MessageBody) -> SendFuture;
    fn try_receive(&self) -> Result<ReceivedMessage, Error>;
    fn receive(&self) -> ReceiveFuture;
    fn call(&self, body: MessageBody) -> Result<CallFuture, Error>;
    fn call_now(&self, body: MessageBody) -> Result<CallTicket, Error>;
    fn close(&self) -> Result<(), Error>;
}

pub(crate) struct SendFuture {
    endpoint: Endpoint,
    message: Option<Message>,
    waiter: Option<core::task::Waker>,
}

pub(crate) struct ReceiveFuture {
    endpoint: Endpoint,
    waiter: Option<core::task::Waker>,
}

pub(crate) struct CallTicket {
    reply: Arc<ReplySlot>,
}

pub(crate) trait CallTicketApi: Sized {
    fn register<const N: usize>(
        &self,
        events: &mut EventLoop<N>,
        data: u64,
    ) -> Result<EventSourceToken, Error>;
    fn try_reply(&self) -> Result<Option<MessageBody>, Error>;
}

pub(crate) struct ReplyCapability {
    slot: Arc<ReplySlot>,
    completed: bool,
}

pub(crate) trait ReplyCapabilityApi: Sized {
    fn reply(self, response: Result<MessageBody, Error>) -> Result<(), Error>;
}

pub(crate) struct CallFuture {
    send: Option<SendFuture>,
    reply: Arc<ReplySlot>,
}

pub(crate) struct SharedMemory {
    core: Arc<SharedMemoryCore>,
    rights: CapabilityRights,
}

pub(crate) trait SharedMemoryApi: Sized {
    fn new(size: usize) -> Result<Self, Error>;
    fn grant(
        &self,
        range: Range<usize>,
        rights: CapabilityRights,
    ) -> Result<SharedMemoryGrant, Error>;
    fn len(&self) -> usize;
}

#[derive(Clone)]
pub(crate) struct SharedMemoryGrant {
    core: Arc<SharedMemoryCore>,
    range: Range<usize>,
    rights: CapabilityRights,
}

pub(crate) trait SharedMemoryGrantApi: Sized {
    fn len(&self) -> usize;
    fn restrict(&self, rights: CapabilityRights) -> Result<Self, Error>;
    fn read(&self, offset: usize, output: &mut [u8]) -> Result<(), Error>;
    fn write(&self, offset: usize, input: &[u8]) -> Result<(), Error>;
    fn map_user(&self, address_space: UserAddressSpace) -> Result<SharedMemoryMapping, Error>;
}

pub(crate) struct SharedMemoryMapping {
    address: usize,
    length: usize,
    address_space: UserAddressSpace,
    _core: Arc<SharedMemoryCore>,
}

pub(crate) trait SharedMemoryMappingApi: Sized {
    fn address(&self) -> usize;
    fn len(&self) -> usize;
}