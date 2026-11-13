/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
	use alloc::vec;

	use super::{Capability, Endpoint, MessageBody};
	use crate::kernel::ipc::{EndpointApi as _, MessageBodyApi as _};

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
