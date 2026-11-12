/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::arch::traits::{
	CpuArchitecture as _, MemoryArchitecture as _, ThreadArchitecture as _,
};
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::vm::UserAddressSpace;

use super::kernel::Context as KernelContext;

pub(crate) struct Context {
	kernel: KernelContext,
	address_space: UserAddressSpace,
	kernel_stack_pointer: usize,
	entry: usize,
	stack_pointer: usize,
}

impl Context {
	pub(crate) fn load(
		kernel_stack: &mut [u8],
		kernel_entry: extern "C" fn() -> !,
		address_space: UserAddressSpace,
		entry: usize,
		stack_pointer: usize,
	) -> Result<Self, Error> {
		let kernel_stack_pointer = (kernel_stack.as_mut_ptr() as usize)
			.checked_add(kernel_stack.len())
			.ok_or(Error::Overflow)?
			& !0xf;
		let stack_address = stack_pointer.checked_sub(1).ok_or(Error::Fault)?;
		if !address_space.user_range_executable(entry, 1) {
			return Err(Error::NoExec);
		}
		if !address_space.user_range_accessible(stack_address, 1, true) {
			return Err(Error::Fault);
		}
		Ok(Self {
			kernel: KernelContext::load(kernel_stack, kernel_entry),
			address_space,
			kernel_stack_pointer,
			entry,
			stack_pointer,
		})
	}

	pub(crate) const fn kernel_stack_pointer(&self) -> usize {
		self.kernel_stack_pointer
	}

	pub(super) fn kernel(&self) -> &KernelContext {
		&self.kernel
	}

	pub(super) fn kernel_mut(&mut self) -> &mut KernelContext {
		&mut self.kernel
	}

	/// Restore this context by entering its validated user entry point.
	///
	/// # Safety
	/// The user mappings must remain owned and mapped for the lifetime of this context.
	pub(crate) unsafe fn restore(&self) -> ! {
		let Some(stack_address) = self.stack_pointer.checked_sub(1) else {
			crate::kernel::arch::CpuArchitecture::halt();
		};
		if !self.address_space.user_range_executable(self.entry, 1)
			|| !self.address_space.user_range_accessible(stack_address, 1, true)
			|| crate::kernel::arch::MemoryArchitecture::current_address_space()
				!= self.address_space.root()
		{
			crate::kernel::arch::CpuArchitecture::halt();
		}
		unsafe {
			crate::kernel::arch::ThreadArchitecture::enter_user(self.entry, self.stack_pointer)
		}
	}
}
