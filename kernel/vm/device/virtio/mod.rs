/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) mod console;
pub(crate) mod block;
pub(crate) mod balloon;
pub(crate) mod fs;
pub(crate) mod gpu;
pub(crate) mod input;
pub(crate) mod mmio;
pub(crate) mod net;
pub(crate) mod queue;
pub(crate) mod rtc;
pub(crate) mod scsi;

use crate::kernel::vm::device::GuestMemory;

const HOST_INPUT_CAPACITY: usize = 512;

struct InputRing {
	bytes: [u8; HOST_INPUT_CAPACITY],
	head: usize,
	length: usize,
}

impl InputRing {
	const EMPTY: Self = Self {
		bytes: [0; HOST_INPUT_CAPACITY],
		head: 0,
		length: 0,
	};

	fn push(&mut self, bytes: &[u8]) {
		for byte in bytes {
			if self.length == HOST_INPUT_CAPACITY {
				break;
			}
			let tail = (self.head + self.length) % HOST_INPUT_CAPACITY;
			self.bytes[tail] = *byte;
			self.length += 1;
		}
	}

	fn read(&mut self, output: &mut [u8]) -> usize {
		let length = output.len().min(self.length);
		let first = length.min(HOST_INPUT_CAPACITY - self.head);
		output[..first].copy_from_slice(&self.bytes[self.head..self.head + first]);
		output[first..length].copy_from_slice(&self.bytes[..length - first]);
		self.head = (self.head + length) % HOST_INPUT_CAPACITY;
		self.length -= length;
		length
	}
}

pub(crate) struct GuestInput {
	console: InputRing,
	keyboard: InputRing,
}

impl GuestInput {
	const fn new() -> Self {
		Self {
			console: InputRing::EMPTY,
			keyboard: InputRing::EMPTY,
		}
	}

	fn pump(&mut self) {
		let mut bytes = [0; 128];
		let length = crate::kernel::driver::virtio::console::Console::read(&mut bytes);
		self.console.push(&bytes[..length]);
		self.keyboard.push(&bytes[..length]);
	}

	pub(crate) fn read_console(&mut self, output: &mut [u8]) -> usize {
		self.console.read(output)
	}

	pub(crate) fn read_keyboard_byte(&mut self) -> Option<u8> {
		let mut byte = [0];
		(self.keyboard.read(&mut byte) == 1).then_some(byte[0])
	}
}

pub(crate) struct GuestVirtioDevices {
	console: console::VirtioConsole,
	rtc: rtc::VirtioRtc,
	block: block::VirtioBlock,
	network: net::VirtioNet,
	scsi: scsi::VirtioScsi,
	filesystem: fs::VirtioFs,
	input: input::VirtioInput,
	gpu: gpu::VirtioGpu,
	balloon: balloon::VirtioBalloon,
	host_input: GuestInput,
}

impl GuestVirtioDevices {
	pub(crate) fn new() -> Result<Self, crate::kernel::arch::VirtualizationError> {
		let block = block::VirtioBlock::new()
			.ok_or(crate::kernel::arch::VirtualizationError::ResourceExhausted)?;
		let network = net::VirtioNet::new()?;
		let scsi = scsi::VirtioScsi::new()
			.ok_or(crate::kernel::arch::VirtualizationError::ResourceExhausted)?;
		let filesystem = fs::VirtioFs::new()
			.ok_or(crate::kernel::arch::VirtualizationError::ResourceExhausted)?;
		let gpu = gpu::VirtioGpu::new()
			.ok_or(crate::kernel::arch::VirtualizationError::ResourceExhausted)?;
		Ok(Self {
			console: console::VirtioConsole::new(),
			rtc: rtc::VirtioRtc::new(),
			block,
			network,
			scsi,
			filesystem,
			input: input::VirtioInput::new(),
			gpu,
			balloon: balloon::VirtioBalloon::new(),
			host_input: GuestInput::new(),
		})
	}

	pub(crate) const fn contains(address: u64) -> bool {
		console::VirtioConsole::contains(address)
			|| rtc::VirtioRtc::contains(address)
			|| block::VirtioBlock::contains(address)
			|| net::VirtioNet::contains(address)
			|| scsi::VirtioScsi::contains(address)
			|| fs::VirtioFs::contains(address)
			|| input::VirtioInput::contains(address)
			|| gpu::VirtioGpu::contains(address)
			|| balloon::VirtioBalloon::contains(address)
	}

	pub(crate) fn access(
		&mut self,
		memory: &mut GuestMemory<'_>,
		address: u64,
		width: u8,
		write: bool,
		value: u64,
	) -> Option<u64> {
		self.host_input.pump();
		if console::VirtioConsole::contains(address) {
			self.console.access(memory, address, width, write, value, &mut self.host_input)
		} else if rtc::VirtioRtc::contains(address) {
			self.rtc.access(memory, address, width, write, value)
		} else if block::VirtioBlock::contains(address) {
			self.block.access(memory, address, width, write, value)
		} else if scsi::VirtioScsi::contains(address) {
			self.scsi.access(memory, address, width, write, value)
		} else if fs::VirtioFs::contains(address) {
			self.filesystem.access(memory, address, width, write, value)
		} else if input::VirtioInput::contains(address) {
			self.input.access(memory, address, width, write, value, &mut self.host_input)
		} else if gpu::VirtioGpu::contains(address) {
			self.gpu.access(memory, address, width, write, value)
		} else if balloon::VirtioBalloon::contains(address) {
			self.balloon.access(memory, address, width, write, value)
		} else {
			self.network.access(memory, address, width, write, value)
		}
	}
}