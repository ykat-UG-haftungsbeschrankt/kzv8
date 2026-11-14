/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::task::Wake;
use alloc::vec::Vec;
use core::future::Future;
use core::ops::Range;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};

use super::{
	CallFuture, CallTicket, CallTicketApi, Capability, CapabilityApi, CapabilityRights,
	CapabilityRightsApi, Endpoint, EndpointApi, MessageBody, MessageBodyApi, ReceiveFuture,
	ReceivedMessage, ReceivedMessageApi, ReplyCapability, ReplyCapabilityApi, SendFuture,
	SharedMemory, SharedMemoryApi, SharedMemoryGrant, SharedMemoryGrantApi, SharedMemoryMapping,
	SharedMemoryMappingApi,
};
use crate::kernel::error::Error;
use crate::kernel::event::{
	EventFlags, EventFlagsApi as _, EventLoop, EventLoopApi as _, EventSignal,
	EventSignalApi as _, EventSourceToken,
};
use crate::kernel::memory::dma::{DmaArena, DmaBuffer};
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::scheduler::thread::vm::UserAddressSpace;

fn record_message_sent() {
	if let Ok(process) = ThreadManager::current_process() {
		process.record_message_sent();
	}
}

fn record_message_received() {
	if let Ok(process) = ThreadManager::current_process() {
		process.record_message_received();
	}
}

impl CapabilityRights {
	const fn is_subset_of(self, rights: Self) -> bool {
		rights.0 & self.0 == self.0
	}
}

impl CapabilityRightsApi for CapabilityRights {
	const NONE: Self = Self(0);
	const SEND: Self = Self(1 << 0);
	const RECEIVE: Self = Self(1 << 1);
	const READ: Self = Self(1 << 2);
	const WRITE: Self = Self(1 << 3);
	const TRANSFER: Self = Self(1 << 4);
	const CLOSE: Self = Self(1 << 5);

	fn union(self, other: Self) -> Self {
		Self(self.0 | other.0)
	}

	fn contains(self, rights: Self) -> bool {
		self.0 & rights.0 == rights.0
	}
}

impl MessageBodyApi for MessageBody {
	fn new(opcode: u32, payload: &[u8]) -> Result<Self, Error> {
		let mut owned_payload = Vec::new();
		owned_payload
			.try_reserve_exact(payload.len())
			.map_err(|_| Error::NoMem)?;
		owned_payload.extend_from_slice(payload);
		Ok(Self {
			opcode,
			payload: owned_payload,
			capabilities: Vec::new(),
		})
	}

	fn opcode(&self) -> u32 {
		self.opcode
	}

	fn payload(&self) -> &[u8] {
		&self.payload
	}

	fn capabilities(&self) -> &[Capability] {
		&self.capabilities
	}

	fn take_capability(&mut self, index: usize) -> Option<Capability> {
		(index < self.capabilities.len()).then(|| self.capabilities.remove(index))
	}

	fn into_parts(self) -> (u32, Vec<u8>, Vec<Capability>) {
		(self.opcode, self.payload, self.capabilities)
	}

	fn push_capability(&mut self, capability: Capability) -> Result<(), Error> {
		self.capabilities.try_reserve(1).map_err(|_| Error::NoMem)?;
		self.capabilities.push(capability);
		Ok(())
	}
}

pub(super) struct Message {
	body: MessageBody,
	reply: Option<ReplyCapability>,
}

impl Message {
	fn one_way(body: MessageBody) -> Self {
		Self { body, reply: None }
	}
}

impl Capability {
	fn transferable(&self) -> bool {
		match self {
			Self::Endpoint(endpoint) => endpoint.rights.contains(CapabilityRights::TRANSFER),
			Self::SharedMemory(grant) => grant.rights.contains(CapabilityRights::TRANSFER),
		}
	}
}

impl CapabilityApi for Capability {
	fn endpoint(&self) -> Option<&Endpoint> {
		match self {
			Self::Endpoint(endpoint) => Some(endpoint),
			Self::SharedMemory(_) => None,
		}
	}

	fn shared_memory(&self) -> Option<&SharedMemoryGrant> {
		match self {
			Self::Endpoint(_) => None,
			Self::SharedMemory(grant) => Some(grant),
		}
	}

	fn duplicate(&self) -> Result<Self, Error> {
		match self {
			Self::Endpoint(endpoint) => Ok(Self::Endpoint(endpoint.clone())),
			Self::SharedMemory(grant) => Ok(Self::SharedMemory(grant.clone())),
		}
	}

	fn restrict(&self, rights: CapabilityRights) -> Result<Self, Error> {
		match self {
			Self::Endpoint(endpoint) => Ok(Self::Endpoint(endpoint.restrict(rights)?)),
			Self::SharedMemory(grant) => Ok(Self::SharedMemory(grant.restrict(rights)?)),
		}
	}

	fn prepare_transfer(&self, rights: CapabilityRights) -> Result<Self, Error> {
		if !self.transferable() || !rights.contains(CapabilityRights::TRANSFER) {
			return Err(Error::Acces);
		}
		self.restrict(rights)
	}
}

impl ReceivedMessageApi for ReceivedMessage {
	fn body(&self) -> &MessageBody {
		&self.body
	}

	fn into_parts(self) -> (MessageBody, Option<ReplyCapability>) {
		(self.body, self.reply)
	}
}

struct EndpointState {
	queue: VecDeque<Message>,
	//@FIXME: INDEXTYPE(send_waiters); deduplicated waiter list scans and retain removal are O(n); keep Vec unless waiter handles enable O(1) unlink.
	send_waiters: Vec<Waker>,
	//@FIXME: INDEXTYPE(receive_waiters); deduplicated waiter list scans and retain removal are O(n); keep Vec unless waiter handles enable O(1) unlink.
	receive_waiters: Vec<Waker>,
	closed: bool,
}

pub(super) struct EndpointCore {
	capacity: usize,
	state: IrqSpinLock<EndpointState>,
	send_signal: EventSignal,
	receive_signal: EventSignal,
}

impl EndpointCore {
	fn new(capacity: usize) -> Result<Self, Error> {
		let mut queue = VecDeque::new();
		queue.try_reserve_exact(capacity).map_err(|_| Error::NoMem)?;
		let send_signal = EventSignal::new();
		send_signal.set_readiness(EventFlags::WRITABLE);
		Ok(Self {
			capacity,
			state: IrqSpinLock::new(EndpointState {
				queue,
				send_waiters: Vec::new(),
				receive_waiters: Vec::new(),
				closed: false,
			}),
			send_signal,
			receive_signal: EventSignal::new(),
		})
	}

	fn update_signal(&self, state: &EndpointState) {
		let mut send_readiness = EventFlags::NONE;
		let mut receive_readiness = EventFlags::NONE;
		if !state.queue.is_empty() {
			receive_readiness = receive_readiness.union(EventFlags::READABLE);
		}
		if !state.closed && state.queue.len() < self.capacity {
			send_readiness = send_readiness.union(EventFlags::WRITABLE);
		}
		if state.closed {
			send_readiness = send_readiness.union(EventFlags::HANGUP);
			receive_readiness = receive_readiness.union(EventFlags::HANGUP);
		}
		self.send_signal.set_readiness(send_readiness);
		self.receive_signal.set_readiness(receive_readiness);
	}

	fn enqueue(&self, message: Message, waiter: Option<&Waker>) -> Result<(), (Error, Message)> {
		if !message
			.body
			.capabilities
			.iter()
			.all(Capability::transferable)
		{
			return Err((Error::Acces, message));
		}

		let mut state = self.state.lock();
		if state.closed {
			return Err((Error::ConnReset, message));
		}
		if state.queue.len() == self.capacity {
			if let Some(waiter) = waiter {
				if let Err(error) = register_waker(&mut state.send_waiters, waiter) {
					return Err((error, message));
				}
			}
			return Err((Error::Again, message));
		}
		if state.queue.try_reserve(1).is_err() {
			return Err((Error::NoMem, message));
		}
		state.queue.push_back(message);
		if let Some(waiter) = waiter {
			remove_waker(&mut state.send_waiters, waiter);
		}
		let waiters = core::mem::take(&mut state.receive_waiters);
		self.update_signal(&state);
		drop(state);
		wake_all(waiters);
		Ok(())
	}

	fn dequeue(&self, waiter: Option<&Waker>) -> Result<Message, Error> {
		let mut state = self.state.lock();
		if let Some(message) = state.queue.pop_front() {
			if let Some(waiter) = waiter {
				remove_waker(&mut state.receive_waiters, waiter);
			}
			let waiters = core::mem::take(&mut state.send_waiters);
			self.update_signal(&state);
			drop(state);
			wake_all(waiters);
			return Ok(message);
		}
		if state.closed {
			return Err(Error::ConnReset);
		}
		if let Some(waiter) = waiter {
			register_waker(&mut state.receive_waiters, waiter)?;
		}
		Err(Error::Again)
	}

	fn close(&self) {
		let mut state = self.state.lock();
		state.closed = true;
		let send_waiters = core::mem::take(&mut state.send_waiters);
		let receive_waiters = core::mem::take(&mut state.receive_waiters);
		self.update_signal(&state);
		drop(state);
		wake_all(send_waiters);
		wake_all(receive_waiters);
	}

	fn remove_send_waiter(&self, waiter: &Waker) {
		remove_waker(&mut self.state.lock().send_waiters, waiter);
	}

	fn remove_receive_waiter(&self, waiter: &Waker) {
		remove_waker(&mut self.state.lock().receive_waiters, waiter);
	}
}

impl EndpointApi for Endpoint {
	fn channel(capacity: usize) -> Result<(Self, Self), Error> {
		if capacity == 0 {
			return Err(Error::Inval);
		}
		let core = Arc::new(EndpointCore::new(capacity)?);
		Ok((
			Self {
				core: Arc::clone(&core),
				rights: CapabilityRights::SEND
					.union(CapabilityRights::TRANSFER)
					.union(CapabilityRights::CLOSE),
			},
			Self {
				core,
				rights: CapabilityRights::RECEIVE
					.union(CapabilityRights::TRANSFER)
					.union(CapabilityRights::CLOSE),
			},
		))
	}

	fn restrict(&self, rights: CapabilityRights) -> Result<Self, Error> {
		if !rights.is_subset_of(self.rights) {
			return Err(Error::Acces);
		}
		Ok(Self {
			core: Arc::clone(&self.core),
			rights,
		})
	}

	fn register<const N: usize>(
		&self,
		events: &mut EventLoop<N>,
		interest: EventFlags,
		data: u64,
	) -> Result<EventSourceToken, Error> {
		match (
			self.rights.contains(CapabilityRights::SEND),
			self.rights.contains(CapabilityRights::RECEIVE),
		) {
			(true, false) => events.register_signal(
				self.core.send_signal.source_id(),
				interest,
				data,
				self.core.send_signal.clone(),
			),
			(false, true) => events.register_signal(
				self.core.receive_signal.source_id(),
				interest,
				data,
				self.core.receive_signal.clone(),
			),
			_ => Err(Error::Inval),
		}
	}

	fn try_send(&self, body: MessageBody) -> Result<(), Error> {
		if !self.rights.contains(CapabilityRights::SEND) {
			return Err(Error::Acces);
		}
		self.core
			.enqueue(Message::one_way(body), None)
			.map_err(|(error, _message)| error)?;
		record_message_sent();
		Ok(())
	}

	fn send(&self, body: MessageBody) -> SendFuture {
		SendFuture {
			endpoint: self.clone(),
			message: Some(Message::one_way(body)),
			waiter: None,
		}
	}

	fn try_receive(&self) -> Result<ReceivedMessage, Error> {
		if !self.rights.contains(CapabilityRights::RECEIVE) {
			return Err(Error::Acces);
		}
		let message = self.core.dequeue(None)?;
		record_message_received();
		Ok(ReceivedMessage {
			body: message.body,
			reply: message.reply,
		})
	}

	fn receive(&self) -> ReceiveFuture {
		ReceiveFuture {
			endpoint: self.clone(),
			waiter: None,
		}
	}

	fn call(&self, body: MessageBody) -> Result<CallFuture, Error> {
		if !self.rights.contains(CapabilityRights::SEND) {
			return Err(Error::Acces);
		}
		let reply = new_reply_slot();
		let message = Message {
			body,
			reply: Some(ReplyCapability {
				slot: Arc::clone(&reply),
				completed: false,
			}),
		};
		Ok(CallFuture {
			send: Some(SendFuture {
				endpoint: self.clone(),
				message: Some(message),
				waiter: None,
			}),
			reply,
		})
	}

	fn call_now(&self, body: MessageBody) -> Result<CallTicket, Error> {
		if !self.rights.contains(CapabilityRights::SEND) {
			return Err(Error::Acces);
		}
		let reply = new_reply_slot();
		let message = Message {
			body,
			reply: Some(ReplyCapability {
				slot: Arc::clone(&reply),
				completed: false,
			}),
		};
		self.core
			.enqueue(message, None)
			.map_err(|(error, _message)| error)?;
		record_message_sent();
		Ok(CallTicket { reply })
	}

	fn close(&self) -> Result<(), Error> {
		if !self.rights.contains(CapabilityRights::CLOSE) {
			return Err(Error::Acces);
		}
		self.core.close();
		Ok(())
	}
}

impl Future for SendFuture {
	type Output = Result<(), Error>;

	fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
		let this = self.get_mut();
		if !this.endpoint.rights.contains(CapabilityRights::SEND) {
			return Poll::Ready(Err(Error::Acces));
		}
		if let Some(waiter) = this.waiter.take() {
			this.endpoint.core.remove_send_waiter(&waiter);
		}
		let Some(message) = this.message.take() else {
			return Poll::Ready(Err(Error::Inval));
		};
		match this.endpoint.core.enqueue(message, Some(context.waker())) {
			Ok(()) => {
				record_message_sent();
				Poll::Ready(Ok(()))
			}
			Err((Error::Again, message)) => {
				this.message = Some(message);
				this.waiter = Some(context.waker().clone());
				Poll::Pending
			}
			Err((error, _message)) => Poll::Ready(Err(error)),
		}
	}
}

impl Drop for SendFuture {
	fn drop(&mut self) {
		if let Some(waiter) = self.waiter.take() {
			self.endpoint.core.remove_send_waiter(&waiter);
		}
	}
}

impl Future for ReceiveFuture {
	type Output = Result<ReceivedMessage, Error>;

	fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
		let this = self.get_mut();
		if !this.endpoint.rights.contains(CapabilityRights::RECEIVE) {
			return Poll::Ready(Err(Error::Acces));
		}
		if let Some(waiter) = this.waiter.take() {
			this.endpoint.core.remove_receive_waiter(&waiter);
		}
		match this.endpoint.core.dequeue(Some(context.waker())) {
			Ok(message) => {
				record_message_received();
				Poll::Ready(Ok(ReceivedMessage {
					body: message.body,
					reply: message.reply,
				}))
			}
			Err(Error::Again) => {
				this.waiter = Some(context.waker().clone());
				Poll::Pending
			}
			Err(error) => Poll::Ready(Err(error)),
		}
	}
}

impl Drop for ReceiveFuture {
	fn drop(&mut self) {
		if let Some(waiter) = self.waiter.take() {
			self.endpoint.core.remove_receive_waiter(&waiter);
		}
	}
}

struct ReplyState {
	response: Option<Result<MessageBody, Error>>,
	waiter: Option<Waker>,
}

pub(super) struct ReplySlot {
	state: IrqSpinLock<ReplyState>,
	signal: EventSignal,
}

fn new_reply_slot() -> Arc<ReplySlot> {
	Arc::new(ReplySlot {
		state: IrqSpinLock::new(ReplyState {
			response: None,
			waiter: None,
		}),
		signal: EventSignal::new(),
	})
}

impl CallTicketApi for CallTicket {
	fn register<const N: usize>(
		&self,
		events: &mut EventLoop<N>,
		data: u64,
	) -> Result<EventSourceToken, Error> {
		events.register_signal(
			self.reply.signal.source_id(),
			EventFlags::READABLE,
			data,
			self.reply.signal.clone(),
		)
	}

	fn try_reply(&self) -> Result<Option<MessageBody>, Error> {
		self.reply.take_response().transpose()
	}
}

impl ReplyCapability {
	fn complete(&self, response: Result<MessageBody, Error>) -> Result<(), Error> {
		let mut state = self.slot.state.lock();
		if state.response.is_some() {
			return Err(Error::Already);
		}
		state.response = Some(response);
		self.slot.signal.set_readiness(EventFlags::READABLE);
		let waiter = state.waiter.take();
		drop(state);
		if let Some(waiter) = waiter {
			waiter.wake();
		}
		Ok(())
	}
}

impl ReplyCapabilityApi for ReplyCapability {
	fn reply(mut self, response: Result<MessageBody, Error>) -> Result<(), Error> {
		let result = self.complete(response);
		self.completed = true;
		result
	}
}

impl ReplySlot {
	fn take_response(&self) -> Option<Result<MessageBody, Error>> {
		let response = self.state.lock().response.take();
		if response.is_some() {
			self.signal.set_readiness(EventFlags::NONE);
		}
		response
	}
}

impl Drop for ReplyCapability {
	fn drop(&mut self) {
		if !self.completed {
			let _ = self.complete(Err(Error::ConnReset));
		}
	}
}

impl Future for CallFuture {
	type Output = Result<MessageBody, Error>;

	fn poll(
		mut self: Pin<&mut Self>,
		context: &mut Context<'_>,
	) -> Poll<Self::Output> {
		if let Some(send) = self.send.as_mut() {
			match Pin::new(send).poll(context) {
				Poll::Ready(Ok(())) => self.send = None,
				Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
				Poll::Pending => return Poll::Pending,
			}
		}
		if let Some(response) = self.reply.take_response() {
			return Poll::Ready(response);
		}
		let mut state = self.reply.state.lock();
		if let Some(response) = state.response.take() {
			self.reply.signal.set_readiness(EventFlags::NONE);
			return Poll::Ready(response);
		}
		state.waiter = Some(context.waker().clone());
		Poll::Pending
	}
}

type AsyncHandler<S> = for<'a> fn(
	&'a mut S,
	MessageBody,
) -> Pin<Box<dyn Future<Output = Result<MessageBody, Error>> + Send + 'a>>;

enum Handler<S> {
	Synchronous(fn(&mut S, MessageBody) -> Result<MessageBody, Error>),
	Asynchronous(AsyncHandler<S>),
}

struct HandlerEntry<S> {
	opcode: u32,
	handler: Handler<S>,
}

impl<S> HandlerEntry<S> {
	const fn synchronous(
		opcode: u32,
		handler: fn(&mut S, MessageBody) -> Result<MessageBody, Error>,
	) -> Self {
		Self {
			opcode,
			handler: Handler::Synchronous(handler),
		}
	}

	const fn asynchronous(opcode: u32, handler: AsyncHandler<S>) -> Self {
		Self {
			opcode,
			handler: Handler::Asynchronous(handler),
		}
	}
}

struct ServiceDispatcher<S, const N: usize> {
	handlers: [HandlerEntry<S>; N],
}

impl<S, const N: usize> ServiceDispatcher<S, N> {
	const fn new(handlers: [HandlerEntry<S>; N]) -> Self {
		Self { handlers }
	}

	fn validate(&self) -> Result<(), Error> {
		for index in 0..N {
			if self.handlers[index + 1..]
				.iter()
				.any(|entry| entry.opcode == self.handlers[index].opcode)
			{
				return Err(Error::Exist);
			}
		}
		Ok(())
	}

	fn handler(&self, opcode: u32) -> Option<&Handler<S>> {
		self.handlers
			.iter()
			.find(|entry| entry.opcode == opcode)
			.map(|entry| &entry.handler)
	}

	fn dispatch_one(&self, endpoint: &Endpoint, state: &mut S) -> Result<bool, Error> {
		let message = match endpoint.try_receive() {
			Ok(message) => message,
			Err(Error::Again) => return Ok(false),
			Err(error) => return Err(error),
		};
		let ReceivedMessage { body, reply } = message;
		let response = match self.handler(body.opcode()) {
			Some(Handler::Synchronous(handler)) => handler(state, body),
			Some(Handler::Asynchronous(_)) => Err(Error::InProgress),
			None => Err(Error::NoSys),
		};
		dispatch_response(reply, response)?;
		Ok(true)
	}

	async fn dispatch_next(&self, endpoint: &Endpoint, state: &mut S) -> Result<(), Error> {
		if !endpoint.rights.contains(CapabilityRights::RECEIVE) {
			return Err(Error::Acces);
		}
		let message = endpoint.receive().await?;
		let ReceivedMessage { body, reply } = message;
		let response = match self.handler(body.opcode()) {
			Some(Handler::Synchronous(handler)) => handler(state, body),
			Some(Handler::Asynchronous(handler)) => handler(state, body).await,
			None => Err(Error::NoSys),
		};
		dispatch_response(reply, response)
	}
}

fn dispatch_response(
	reply: Option<ReplyCapability>,
	response: Result<MessageBody, Error>,
) -> Result<(), Error> {
	if let Some(reply) = reply {
		reply.reply(response)
	} else {
		response.map(|_| ())
	}
}

pub(super) struct SharedMemoryCore {
	buffer: IrqSpinLock<DmaBuffer>,
	length: usize,
}

impl SharedMemoryApi for SharedMemory {
	fn new(size: usize) -> Result<Self, Error> {
		if size == 0 {
			return Err(Error::Inval);
		}
		let length = size
			.checked_add(crate::kernel::memory::r#virtual::PAGE_SIZE - 1)
			.map(|length| length & !(crate::kernel::memory::r#virtual::PAGE_SIZE - 1))
			.ok_or(Error::Overflow)?;
		let mut buffer = DmaArena::allocate(length, crate::kernel::memory::r#virtual::PAGE_SIZE)
			.ok_or(Error::NoMem)?;
		buffer.as_mut_slice().fill(0);
		Ok(Self {
			core: Arc::new(SharedMemoryCore {
				buffer: IrqSpinLock::new(buffer),
				length,
			}),
			rights: CapabilityRights::READ
				.union(CapabilityRights::WRITE)
				.union(CapabilityRights::TRANSFER),
		})
	}

	fn grant(
		&self,
		range: Range<usize>,
		rights: CapabilityRights,
	) -> Result<SharedMemoryGrant, Error> {
		let size = self.core.length;
		if range.start >= range.end || range.end > size {
			return Err(Error::Inval);
		}
		if !rights.is_subset_of(self.rights)
			|| rights.contains(CapabilityRights::SEND)
			|| rights.contains(CapabilityRights::RECEIVE)
		{
			return Err(Error::Acces);
		}
		Ok(SharedMemoryGrant {
			core: Arc::clone(&self.core),
			range,
			rights,
		})
	}

	fn len(&self) -> usize {
		self.core.length
	}
}

impl SharedMemoryGrantApi for SharedMemoryGrant {
	fn len(&self) -> usize {
		self.range.end - self.range.start
	}

	fn restrict(&self, rights: CapabilityRights) -> Result<Self, Error> {
		if !rights.is_subset_of(self.rights) {
			return Err(Error::Acces);
		}
		Ok(Self {
			core: Arc::clone(&self.core),
			range: self.range.clone(),
			rights,
		})
	}

	fn read(&self, offset: usize, output: &mut [u8]) -> Result<(), Error> {
		if !self.rights.contains(CapabilityRights::READ) {
			return Err(Error::Acces);
		}
		let end = offset.checked_add(output.len()).ok_or(Error::Inval)?;
		if end > self.len() {
			return Err(Error::Fault);
		}
		let mut buffer = self.core.buffer.lock();
		let bytes = buffer.as_mut_slice();
		let start = self.range.start + offset;
		output.copy_from_slice(&bytes[start..start + output.len()]);
		Ok(())
	}

	fn write(&self, offset: usize, input: &[u8]) -> Result<(), Error> {
		if !self.rights.contains(CapabilityRights::WRITE) {
			return Err(Error::Acces);
		}
		let end = offset.checked_add(input.len()).ok_or(Error::Inval)?;
		if end > self.len() {
			return Err(Error::Fault);
		}
		let mut buffer = self.core.buffer.lock();
		let bytes = buffer.as_mut_slice();
		let start = self.range.start + offset;
		bytes[start..start + input.len()].copy_from_slice(input);
		Ok(())
	}

	fn map_user(&self, address_space: UserAddressSpace) -> Result<SharedMemoryMapping, Error> {
		const PAGE_SIZE: usize = crate::kernel::memory::r#virtual::PAGE_SIZE;
		let length = self.len();
		if self.range.start % PAGE_SIZE != 0 || length % PAGE_SIZE != 0 {
			return Err(Error::Inval);
		}
		let read = self.rights.contains(CapabilityRights::READ);
		let write = self.rights.contains(CapabilityRights::WRITE);
		if !read && !write {
			return Err(Error::Acces);
		}
		let permissions = if write { 2 } else { 1 };
		let address = address_space
			.reserve_user_mapping(None, length, permissions, false)
			.ok_or(Error::NoMem)?;
		let physical_base = self.core.buffer.lock().bus_address() as usize;
		for offset in (0..length).step_by(PAGE_SIZE) {
			let Some(physical_address) = physical_base
				.checked_add(self.range.start)
				.and_then(|base| base.checked_add(offset))
			else {
				let _released = address_space.release_user_mapping(address, length);
				return Err(Error::Overflow);
			};
			if !address_space.map_user_borrowed_page(
				address + offset,
				physical_address,
				permissions,
			) {
				let _released = address_space.release_user_mapping(address, length);
				return Err(Error::NoMem);
			}
		}
		Ok(SharedMemoryMapping {
			address,
			length,
			address_space,
			_core: Arc::clone(&self.core),
		})
	}
}

impl SharedMemoryMappingApi for SharedMemoryMapping {
	fn address(&self) -> usize {
		self.address
	}

	fn len(&self) -> usize {
		self.length
	}
}

impl Drop for SharedMemoryMapping {
	fn drop(&mut self) {
		let _released = self
			.address_space
			.release_user_mapping(self.address, self.length);
	}
}

fn register_waker(waiters: &mut Vec<Waker>, waker: &Waker) -> Result<(), Error> {
	if waiters.iter().any(|waiter| waiter.will_wake(waker)) {
		return Ok(());
	}
	waiters.try_reserve(1).map_err(|_| Error::NoMem)?;
	waiters.push(waker.clone());
	Ok(())
}

fn remove_waker(waiters: &mut Vec<Waker>, waker: &Waker) {
	waiters.retain(|waiter| !waiter.will_wake(waker));
}

fn wake_all(waiters: Vec<Waker>) {
	for waiter in waiters {
		waiter.wake();
	}
}

struct IpcSmokeWake;

impl Wake for IpcSmokeWake {
	fn wake(self: Arc<Self>) {}
}

fn ipc_smoke_echo(state: &mut usize, body: MessageBody) -> Result<MessageBody, Error> {
	*state += 1;
	if let Some(grant) = body
		.capabilities()
		.first()
		.and_then(Capability::shared_memory)
	{
		let mut bytes = [0u8; 4];
		grant.read(0, &mut bytes)?;
		MessageBody::new(body.opcode(), &bytes[..])
	} else {
		MessageBody::new(body.opcode(), body.payload())
	}
}

fn ipc_smoke_async_echo<'a>(
	state: &'a mut usize,
	body: MessageBody,
) -> Pin<Box<dyn Future<Output = Result<MessageBody, Error>> + Send + 'a>> {
	Box::pin(async move {
		*state += 1;
		MessageBody::new(body.opcode(), body.payload())
	})
}

fn ipc_smoke_async_error<'a>(
	_state: &'a mut usize,
	_body: MessageBody,
) -> Pin<Box<dyn Future<Output = Result<MessageBody, Error>> + Send + 'a>> {
	Box::pin(async { Err(Error::Io) })
}

pub(crate) fn process_capability_smoke() -> Result<(), Error> {
	use crate::kernel::process::Process;

	let sender_process = Process::new()?;
	let receiver_process = Process::new()?;
	if sender_process.id() == receiver_process.id() {
		return Err(Error::Proto);
	}
	let (transport_sender, transport_receiver) = Endpoint::channel(4)?;
	let transport_sender =
		sender_process.insert_capability(Capability::Endpoint(transport_sender))?;
	let transport_receiver =
		receiver_process.insert_capability(Capability::Endpoint(transport_receiver))?;

	let (service_sender, service_receiver) = sender_process.create_endpoint_pair(1)?;
	let reduced_rights = CapabilityRights::SEND.union(CapabilityRights::TRANSFER);
	let reduced_sender = sender_process.restrict_capability(service_sender, reduced_rights)?;
	let mut endpoint_body = MessageBody::new(30, b"service endpoint")?;
	sender_process.attach_capability(&mut endpoint_body, reduced_sender, reduced_rights)?;
	sender_process.try_send(transport_sender, endpoint_body)?;
	let received_endpoint = receiver_process.try_receive(transport_receiver)?;
	if received_endpoint.payload() != b"service endpoint" {
		return Err(Error::Proto);
	}
	let received_endpoint_handle = *received_endpoint.capabilities().first().ok_or(Error::Proto)?;
	let transferred = receiver_process.capability(received_endpoint_handle)?;
	let Capability::Endpoint(transferred_endpoint) = transferred else {
		return Err(Error::Proto);
	};
	if !matches!(transferred_endpoint.close(), Err(Error::Acces))
		|| !matches!(
			sender_process.prepare_transfer(
				reduced_sender,
				CapabilityRights::SEND
					.union(CapabilityRights::TRANSFER)
					.union(CapabilityRights::CLOSE),
			),
			Err(Error::Acces)
		)
	{
		return Err(Error::Proto);
	}
	receiver_process.try_send(
		received_endpoint_handle,
		MessageBody::new(31, b"delegated")?,
	)?;
	if sender_process.try_receive(service_receiver)?.payload() != b"delegated" {
		return Err(Error::Proto);
	}
	sender_process.close_capability(service_receiver)?;
	sender_process.close_capability(reduced_sender)?;
	if !matches!(sender_process.capability(reduced_sender), Err(Error::Badf)) {
		return Err(Error::Proto);
	}

	Ok(())
}

pub(crate) fn shared_mapping_smoke() -> Result<(), Error> {
	use crate::kernel::process::Process;

	let sender_process = Process::new()?;
	let receiver_process = Process::new()?;
	let (transport_sender, transport_receiver) = Endpoint::channel(1)?;
	let transport_sender =
		sender_process.insert_capability(Capability::Endpoint(transport_sender))?;
	let transport_receiver =
		receiver_process.insert_capability(Capability::Endpoint(transport_receiver))?;
	let page_size = crate::kernel::memory::r#virtual::PAGE_SIZE;
	let shared_memory = SharedMemory::new(page_size)?;
	let full_rights = CapabilityRights::READ
		.union(CapabilityRights::WRITE)
		.union(CapabilityRights::TRANSFER);
	let grant = shared_memory.grant(0..page_size, full_rights)?;
	grant.write(0, b"page")?;
	let grant_handle = sender_process.insert_capability(Capability::SharedMemory(grant))?;
	let read_only_rights = CapabilityRights::READ.union(CapabilityRights::TRANSFER);
	let mut grant_body = MessageBody::new(32, b"read-only grant")?;
	sender_process.attach_capability(&mut grant_body, grant_handle, read_only_rights)?;
	sender_process.try_send(transport_sender, grant_body)?;
	let received_grant = receiver_process.try_receive(transport_receiver)?;
	let received_grant_handle = *received_grant.capabilities().first().ok_or(Error::Proto)?;
	let mapping = receiver_process.map_shared_memory(received_grant_handle)?;
	if mapping.len() != page_size
		|| !mapping
			.address_space
			.user_range_accessible(mapping.address(), 4, false)
		|| mapping
			.address_space
			.user_range_accessible(mapping.address(), 1, true)
	{
		return Err(Error::Proto);
	}
	let mut mapped_bytes = [0; 4];
	crate::kernel::dl::AddressSpace::read(
		&mapping.address_space,
		mapping.address(),
		&mut mapped_bytes,
	)?;
	if &mapped_bytes != b"page" {
		return Err(Error::Proto);
	}
	receiver_process.close_capability(received_grant_handle)?;
	drop(mapping);
	Ok(())
}

pub(crate) fn smoke() -> Result<(), Error> {
	use crate::kernel::event::{Event, EventFlags, EventFlagsApi as _, EventLoop, EventLoopApi as _};

	let waker = Waker::from(Arc::new(IpcSmokeWake));
	let mut context = Context::from_waker(&waker);

	let (client, server) = Endpoint::channel(1)?;
	let mut events = EventLoop::<4>::new();
	let _send_source = client.register(&mut events, EventFlags::WRITABLE, 16)?;
	let _receive_source = server.register(&mut events, EventFlags::READABLE, 17)?;
	if !matches!(
		client.register(&mut events, EventFlags::WRITABLE, 18),
		Err(Error::Exist)
	) {
		return Err(Error::Proto);
	}
	if !matches!(
		events.poll_readiness(),
		Some(Event::Readiness { flags, .. }) if flags == EventFlags::WRITABLE
	) {
		return Err(Error::Io);
	}
	let receive_only = client.restrict(CapabilityRights::CLOSE)?;
	if !matches!(
		receive_only.try_send(MessageBody::new(1, b"denied")?),
		Err(Error::Acces)
	) || !matches!(
		receive_only.register(&mut events, EventFlags::READABLE, 19),
		Err(Error::Inval)
	) {
		return Err(Error::Proto);
	}
	if !matches!(
		client.restrict(CapabilityRights::SEND.union(CapabilityRights::RECEIVE)),
		Err(Error::Acces)
	) {
		return Err(Error::Proto);
	}
	client.try_send(MessageBody::new(1, b"ping")?)?;
	if !matches!(
		events.poll_readiness(),
		Some(Event::Readiness { flags, .. }) if flags == EventFlags::READABLE
	) {
		return Err(Error::Io);
	}
	let message = server.try_receive()?;
	if message.body().payload() != b"ping" {
		return Err(Error::Proto);
	}

	client.try_send(MessageBody::new(2, b"one")?)?;
	if !matches!(client.try_send(MessageBody::new(2, b"two")?), Err(Error::Again)) {
		return Err(Error::Proto);
	}
	if !matches!(
		events.poll_readiness(),
		Some(Event::Readiness { flags, .. }) if flags == EventFlags::READABLE
	) {
		return Err(Error::Proto);
	}
	if server.try_receive()?.body().payload() != b"one" {
		return Err(Error::Proto);
	}

	let (async_client, async_server) = Endpoint::channel(1)?;
	let mut receive = Box::pin(async_server.receive());
	if !matches!(receive.as_mut().poll(&mut context), Poll::Pending) {
		return Err(Error::Proto);
	}
	async_client.try_send(MessageBody::new(3, b"wake")?)?;
	let received = match receive.as_mut().poll(&mut context) {
		Poll::Ready(Ok(message)) => message,
		Poll::Ready(Err(error)) => return Err(error),
		Poll::Pending => return Err(Error::Io),
	};
	if received.body().payload() != b"wake" {
		return Err(Error::Proto);
	}

	let (closing_sender, closing_receiver) = Endpoint::channel(1)?;
	let mut closed_receive = Box::pin(closing_receiver.receive());
	if !matches!(closed_receive.as_mut().poll(&mut context), Poll::Pending) {
		return Err(Error::Proto);
	}
	closing_sender.close()?;
	if !matches!(
		closed_receive.as_mut().poll(&mut context),
		Poll::Ready(Err(Error::ConnReset))
	) {
		return Err(Error::Proto);
	}
	drop(closed_receive);

	let (full_sender, full_receiver) = Endpoint::channel(1)?;
	full_sender.try_send(MessageBody::new(8, b"full")?)?;
	let mut closed_send = Box::pin(full_sender.send(MessageBody::new(8, b"blocked")?));
	if !matches!(closed_send.as_mut().poll(&mut context), Poll::Pending) {
		return Err(Error::Proto);
	}
	full_receiver.close()?;
	if !matches!(
		closed_send.as_mut().poll(&mut context),
		Poll::Ready(Err(Error::ConnReset))
	) {
		return Err(Error::Proto);
	}
	drop(closed_send);

	let (orphan_client, orphan_server) = Endpoint::channel(1)?;
	let mut orphan_call = Box::pin(orphan_client.call(MessageBody::new(7, b"orphan")?)?);
	if !matches!(orphan_call.as_mut().poll(&mut context), Poll::Pending) {
		return Err(Error::Proto);
	}
	drop(orphan_server.try_receive()?);
	if !matches!(
		orphan_call.as_mut().poll(&mut context),
		Poll::Ready(Err(Error::ConnReset))
	) {
		return Err(Error::Proto);
	}

	let (async_sender, async_receiver) = Endpoint::channel(1)?;
	async_sender.try_send(MessageBody::new(4, b"full")?)?;
	let mut send = Box::pin(async_sender.send(MessageBody::new(4, b"space")?));
	if !matches!(send.as_mut().poll(&mut context), Poll::Pending) {
		return Err(Error::Proto);
	}
	if async_receiver.try_receive()?.body().payload() != b"full" {
		return Err(Error::Proto);
	}
	if !matches!(send.as_mut().poll(&mut context), Poll::Ready(Ok(())))
		|| async_receiver.try_receive()?.body().payload() != b"space"
	{
		return Err(Error::Io);
	}

	let (transferred_endpoint, transferred_receiver) = Endpoint::channel(1)?;
	let mut endpoint_message = MessageBody::new(5, b"cap")?;
	endpoint_message.push_capability(Capability::Endpoint(transferred_endpoint))?;
	client.try_send(endpoint_message)?;
	let received_endpoint = server.try_receive()?;
	let endpoint_capability = received_endpoint
		.body()
		.capabilities()
		.first()
		.and_then(Capability::endpoint)
		.ok_or(Error::Proto)?;
	endpoint_capability.try_send(MessageBody::new(6, b"delegated")?)?;
	if transferred_receiver.try_receive()?.body().payload() != b"delegated" {
		return Err(Error::Proto);
	}

	let dispatcher = ServiceDispatcher::<usize, 3>::new([
		HandlerEntry::synchronous(10, ipc_smoke_echo),
		HandlerEntry::asynchronous(11, ipc_smoke_async_echo),
		HandlerEntry::asynchronous(12, ipc_smoke_async_error),
	]);
	dispatcher.validate()?;
	let duplicate_dispatcher = ServiceDispatcher::<usize, 2>::new([
		HandlerEntry::synchronous(20, ipc_smoke_echo),
		HandlerEntry::synchronous(20, ipc_smoke_echo),
	]);
	if !matches!(duplicate_dispatcher.validate(), Err(Error::Exist)) {
		return Err(Error::Proto);
	}
	let (call_client, call_server) = Endpoint::channel(2)?;
	let mut call_state = 0;
	let shared_memory = SharedMemory::new(8)?;
	let grant_rights = CapabilityRights::READ
		.union(CapabilityRights::WRITE)
		.union(CapabilityRights::TRANSFER);
	let grant = shared_memory.grant(2..6, grant_rights)?;
	grant.write(0, b"data")?;
	let mut request = MessageBody::new(10, b"grant")?;
	request.push_capability(Capability::SharedMemory(grant))?;
	let ticket = call_client.call_now(request)?;
	let mut reply_events = EventLoop::<4>::new();
	let _reply_source = ticket.register(&mut reply_events, 23)?;
	if !dispatcher.dispatch_one(&call_server, &mut call_state)? {
		return Err(Error::Io);
	}
	if reply_events.poll_readiness().is_none() {
		return Err(Error::Io);
	}
	let response = ticket.try_reply()?.ok_or(Error::Io)?;
	if response.payload() != b"data" || call_state != 1 {
		return Err(Error::Proto);
	}

	let mut async_call = Box::pin(call_client.call(MessageBody::new(11, b"async")?)?);
	if !matches!(async_call.as_mut().poll(&mut context), Poll::Pending) {
		return Err(Error::Proto);
	}
	let mut dispatch = Box::pin(dispatcher.dispatch_next(&call_server, &mut call_state));
	if !matches!(dispatch.as_mut().poll(&mut context), Poll::Ready(Ok(()))) {
		return Err(Error::Io);
	}
	drop(dispatch);
	let response = match async_call.as_mut().poll(&mut context) {
		Poll::Ready(Ok(response)) => response,
		Poll::Ready(Err(error)) => return Err(error),
		Poll::Pending => return Err(Error::Io),
	};
	if response.payload() != b"async" || call_state != 2 {
		return Err(Error::Proto);
	}

	let mut error_call = Box::pin(call_client.call(MessageBody::new(12, b"error")?)?);
	if !matches!(error_call.as_mut().poll(&mut context), Poll::Pending) {
		return Err(Error::Proto);
	}
	let mut error_dispatch = Box::pin(dispatcher.dispatch_next(&call_server, &mut call_state));
	if !matches!(error_dispatch.as_mut().poll(&mut context), Poll::Ready(Ok(()))) {
		return Err(Error::Io);
	}
	drop(error_dispatch);
	if !matches!(
		error_call.as_mut().poll(&mut context),
		Poll::Ready(Err(Error::Io))
	) {
		return Err(Error::Proto);
	}
	Ok(())
}

#[cfg(test)]
#[path = "../../tests/kernel/ipc/ipc.rs"]
mod dynamic_capacity_tests;
