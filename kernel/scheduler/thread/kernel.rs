/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::arch::traits::ThreadArchitecture as _;

use super::FpuContext;

pub(crate) struct Context {
	pub(super) stack_pointer: usize,
	pub(super) fpu_context: FpuContext,
}

impl Context {
	pub(crate) const fn main() -> Self {
		Self {
			stack_pointer: 0,
			fpu_context: FpuContext::INITIAL,
		}
	}

	pub(crate) fn load(stack: &mut [u8], entry: extern "C" fn() -> !) -> Self {
		Self {
			stack_pointer: crate::kernel::arch::ThreadArchitecture::initialize_thread_stack(
				stack, entry,
			),
			fpu_context: FpuContext::INITIAL,
		}
	}

	pub(crate) unsafe fn restore(&mut self, next: &Self) {
		unsafe {
			crate::kernel::arch::ThreadArchitecture::switch_thread(
				&mut self.stack_pointer,
				next.stack_pointer,
				&mut self.fpu_context,
				&next.fpu_context,
			);
		}
	}
}
