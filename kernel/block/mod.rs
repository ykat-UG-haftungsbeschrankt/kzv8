/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) mod device;
pub(crate) mod request;
pub(crate) mod service;

pub(crate) use device::BlockDevice;
pub(crate) use request::{Operation, Request};
pub(crate) use service::{BlockClient, BlockService};

use crate::kernel::error::Error;

pub(crate) trait BlockDeviceApi {
	fn initialize() -> Option<Self>
	where
		Self: Sized;

	fn capacity_sectors(&self) -> u64;
	#[allow(dead_code)]
	fn interrupt_slot(&self) -> usize;
	fn submit<'a>(
		&'a self,
		request: Request<'a>,
	) -> Result<crate::kernel::driver::virtio::block::BlockTransfer<'a>, Error>;
}

pub(crate) trait RequestApi<'a>: Sized {
	fn read(sector: u64, output: &'a mut [u8]) -> Self;
	fn write(sector: u64, input: &'a [u8]) -> Self;
}

pub(crate) trait BlockClientApi {
	fn capacity_sectors(&self) -> u64;
	fn smoke(&self) -> Result<(), Error>;
	fn read(&self, sector: u64, output: &mut [u8]) -> Result<(), Error>;
	fn write(&self, sector: u64, input: &[u8]) -> Result<(), Error>;
}

pub(crate) trait BlockServiceApi: Sized {
	fn start(device: BlockDevice) -> Option<BlockClient>;
	#[allow(dead_code)]
	fn ready_marker(client: &Option<BlockClient>) -> &'static str;
}
