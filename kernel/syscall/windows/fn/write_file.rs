/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::dl::AddressSpace as _;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

const MAX_WRITE_LENGTH: usize = 1 << 20;
const STACK_ARGUMENT_COUNT: usize = 5;
const STACK_ARGUMENT_OFFSET: usize = 5 * core::mem::size_of::<u64>();

pub(crate) struct WriteFile;

impl WriteFile {
	pub(crate) const ID: u64 = 0x0008;

	pub(crate) fn handle(frame: SyscallFrame, user_rsp: usize) -> Result<u32, Error> {
		const PSEUDO_STDOUT: u64 = 1;
		const PSEUDO_STDERR: u64 = 2;

		if !matches!(frame.windows_arguments[0], PSEUDO_STDOUT | PSEUDO_STDERR) {
			return Err(Error::Badf);
		}
		if frame.windows_arguments[1..4].iter().any(|argument| *argument != 0) {
			return Err(Error::OpNotSupp);
		}

		let stack_arguments = read_stack_arguments(user_rsp)?;
		let io_status_address = usize::try_from(stack_arguments[0]).map_err(|_| Error::Fault)?;
		let buffer_address = usize::try_from(stack_arguments[1]).map_err(|_| Error::Fault)?;
		let length = usize::try_from(stack_arguments[2]).map_err(|_| Error::Fbig)?;
		if stack_arguments[3] != 0 || stack_arguments[4] != 0 {
			return Err(Error::OpNotSupp);
		}
		if length > MAX_WRITE_LENGTH {
			return Err(Error::Fbig);
		}
		if io_status_address == 0 {
			return Err(Error::Fault);
		}

		let process = ThreadManager::current_process()?;
		let mut address_space = process.address_space();
		if !address_space.user_range_accessible(io_status_address, 16, true)
			|| (length != 0 && !address_space.user_range_accessible(buffer_address, length, false))
		{
			return Err(Error::Fault);
		}
		let mut buffer = alloc::vec::Vec::new();
		buffer.try_reserve_exact(length).map_err(|_| Error::NoMem)?;
		buffer.resize(length, 0);
		if length != 0 {
			address_space
				.read(buffer_address, &mut buffer)
				.map_err(|_| Error::Fault)?;
		}
		let written = crate::kernel::tty::write_console(&buffer, false)?;
		let mut io_status = [0; 16];
		io_status[8..16].copy_from_slice(&(written as u64).to_le_bytes());
		address_space
			.write(io_status_address, &io_status)
			.map_err(|_| Error::Fault)?;
		Ok(0)
	}
}

fn read_stack_arguments(user_rsp: usize) -> Result<[u64; STACK_ARGUMENT_COUNT], Error> {
	let address = user_rsp
		.checked_add(STACK_ARGUMENT_OFFSET)
		.ok_or(Error::Fault)?;
	let mut bytes = [0; STACK_ARGUMENT_COUNT * core::mem::size_of::<u64>()];
	ThreadManager::current_process()?
		.address_space()
		.read(address, &mut bytes)
		.map_err(|_| Error::Fault)?;
	let mut arguments = [0; STACK_ARGUMENT_COUNT];
	for (index, chunk) in bytes.chunks_exact(8).enumerate() {
		arguments[index] = u64::from_ne_bytes(chunk.try_into().map_err(|_| Error::Fault)?);
	}
	Ok(arguments)
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/windows/fn/write_file.rs"]
mod tests;