/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

const PAGE_SIZE: usize = crate::kernel::memory::r#virtual::PAGE_SIZE;

pub(crate) struct Brk;

impl Brk {
	pub(crate) const ID: u64 = crate::kernel::arch::syscall::Brk;

	pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
		let requested = usize::try_from(frame.arguments[0]).map_err(|_| Error::Inval)?;
		let process = ThreadManager::current_process()?;
		let address_space = process.address_space();
		let (base, current, mapped_end) = address_space.program_break_state();
		if base == 0 || requested == 0 || requested < base {
			return i64::try_from(current).map_err(|_| Error::Overflow);
		}

		let Some(required_end) = requested
			.checked_add(PAGE_SIZE - 1)
			.map(|end| end & !(PAGE_SIZE - 1))
		else {
			return i64::try_from(current).map_err(|_| Error::Overflow);
		};
		let mut mapped_end = mapped_end;
		if required_end > mapped_end {
			let length = required_end - mapped_end;
			let Ok(bytes) = u64::try_from(length) else {
				return i64::try_from(current).map_err(|_| Error::Overflow);
			};
			if process.try_reserve_memory(bytes).is_err() {
				return i64::try_from(current).map_err(|_| Error::Overflow);
			}
			if address_space
				.reserve_user_mapping(Some(mapped_end), length, 2, true)
				!= Some(mapped_end)
			{
				process.release_memory(bytes, bytes, 0);
				return i64::try_from(current).map_err(|_| Error::Overflow);
			}
			mapped_end = required_end;
		}

		address_space.set_program_break_state((base, requested, mapped_end));
		i64::try_from(requested).map_err(|_| Error::Overflow)
	}
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/brk.rs"]
mod tests;