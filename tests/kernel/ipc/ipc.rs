/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
	use alloc::vec;
	use alloc::sync::Arc;
	use alloc::task::Wake;
	use core::future::Future;
	use core::task::{Context, Poll, Waker};

	use super::{Capability, Endpoint, MessageBody};
	use crate::kernel::ipc::{EndpointApi as _, MessageBodyApi as _};

	struct TestWake;

	impl Wake for TestWake {
		fn wake(self: Arc<Self>) {}
	}

	#[test]
	fn message_payload_and_capabilities_grow_dynamically() {
		let payload = vec![0x5a; 4096];
		let mut message = MessageBody::new(1, &payload).unwrap();
		assert_eq!(message.payload(), payload.as_slice());

		for _ in 0..8 {
			let (endpoint, _receiver) = Endpoint::channel(1).unwrap();
			message
				.push_capability(Capability::Endpoint(endpoint))
				.unwrap();
		}
		assert_eq!(message.capabilities().len(), 8);
	}

	#[test]
	fn endpoint_queue_capacity_can_exceed_previous_global_limit() {
		let (sender, receiver) = Endpoint::channel(1100).unwrap();
		for _ in 0..1100 {
			sender.try_send(MessageBody::new(2, b"queued").unwrap()).unwrap();
		}
		assert_eq!(receiver.core.state.lock().queue.len(), 1100);
	}

	#[test]
	fn send_waiter_nodes_promote_unlink_and_complete() {
		let (sender, receiver) = Endpoint::channel(1).unwrap();
		sender.try_send(MessageBody::new(1, b"full").unwrap()).unwrap();
		let waker = Waker::from(Arc::new(TestWake));
		let mut context = Context::from_waker(&waker);
		let mut first = Box::pin(sender.send(MessageBody::new(2, b"first").unwrap()));
		let mut second = Box::pin(sender.send(MessageBody::new(3, b"second").unwrap()));

		assert!(matches!(first.as_mut().poll(&mut context), Poll::Pending));
		assert!(matches!(second.as_mut().poll(&mut context), Poll::Pending));
		{
			let state = sender.core.state.lock();
			assert!(state.send_waiting.head.is_some());
			assert!(state.send_ready.head.is_none());
		}

		receiver.try_receive().unwrap();
		{
			let state = sender.core.state.lock();
			assert!(state.send_waiting.head.is_none());
			assert!(state.send_ready.head.is_some());
		}

		drop(first);
		{
			let state = sender.core.state.lock();
			assert!(state.send_ready.head.is_some());
			assert_eq!(state.send_ready.head, state.send_ready.tail);
		}

		assert!(matches!(second.as_mut().poll(&mut context), Poll::Ready(Ok(()))));
		let state = sender.core.state.lock();
		assert!(state.send_ready.head.is_none());
		assert!(state.send_waiting.head.is_none());
	}

	#[test]
	fn closing_endpoint_wakes_and_unlinks_waiting_sender() {
		let (sender, _receiver) = Endpoint::channel(1).unwrap();
		sender.try_send(MessageBody::new(1, b"full").unwrap()).unwrap();
		let waker = Waker::from(Arc::new(TestWake));
		let mut context = Context::from_waker(&waker);
		let mut send = Box::pin(sender.send(MessageBody::new(2, b"blocked").unwrap()));

		assert!(matches!(send.as_mut().poll(&mut context), Poll::Pending));
		sender.close().unwrap();
		assert!(matches!(
			send.as_mut().poll(&mut context),
			Poll::Ready(Err(crate::kernel::error::Error::ConnReset))
		));

		let state = sender.core.state.lock();
		assert!(state.send_ready.head.is_none());
		assert!(state.send_waiting.head.is_none());
	}
