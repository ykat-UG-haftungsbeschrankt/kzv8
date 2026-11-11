/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::ptr::NonNull;

use crate::kernel::dl::{AddressSpace, ElfKind, Error, LoadedObject, Permissions};
use crate::kernel::arch::traits::{CpuArchitecture as _, ThreadArchitecture as _};
use crate::kernel::lib::alignment::align_up;
#[cfg(feature = "virtualization")]
use crate::kernel::vm::device::passthru::{PassthroughDevice, PciFunction};

#[cfg(feature = "virtualization")]
pub(crate) struct VcpuContext {
	kernel: super::kernel::Context,
	passthrough: Vec<PassthroughDevice>,
	guest: <crate::kernel::arch::VirtualizationArchitecture as crate::kernel::arch::traits::VirtualizationArchitecture>::GuestContext,
}

#[cfg(feature = "virtualization")]
impl VcpuContext {
	pub(crate) fn load(
		kernel_stack: &mut [u8],
		kernel_entry: extern "C" fn() -> !,
		image: &[u8],
	) -> Result<Self, crate::kernel::arch::VirtualizationError> {
		let guest = crate::kernel::arch::VirtualizationArchitecture::load_guest_context(image)?;
		let kernel = super::kernel::Context::load(kernel_stack, kernel_entry);
		Ok(Self {
			kernel,
			passthrough: Vec::new(),
			guest,
		})
	}

	pub(crate) fn load_with_pci_function(
		kernel_stack: &mut [u8],
		kernel_entry: extern "C" fn() -> !,
		image: &[u8],
		function: PciFunction,
	) -> Result<Self, crate::kernel::arch::VirtualizationError> {
		let mut context = Self::load(kernel_stack, kernel_entry, image)?;
		context
			.passthrough
			.try_reserve(1)
			.map_err(|_| crate::kernel::arch::VirtualizationError::ResourceExhausted)?;
		let device = PassthroughDevice::attach(&mut context.guest, function)?;
		context.passthrough.push(device);
		Ok(context)
	}

	pub(super) fn kernel(&self) -> &super::kernel::Context {
		&self.kernel
	}

	pub(super) fn kernel_mut(&mut self) -> &mut super::kernel::Context {
		&mut self.kernel
	}

	/// Restore guest execution until the architecture reports a VM exit.
	///
	/// # Safety
	/// The guest image loaded into this context must be trusted.
	unsafe fn restore(
		&mut self,
	) -> Result<crate::kernel::arch::VirtualMachineExit, crate::kernel::arch::VirtualizationError>
	{
		// SAFETY: the context was created by the architecture backend from this guest image.
		unsafe {
			crate::kernel::arch::VirtualizationArchitecture::restore_guest_context(&mut self.guest)
		}
	}

	/// Run this vCPU in slices, yielding to the thread scheduler at VM preemption exits.
	///
	/// # Safety
	/// Only trusted guest images may be resumed by the architecture backend.
	/// Run until this VM thread exits; save the vCPU state at every yielding VM exit.
	///
	/// # Safety
	/// The pointer must refer to the current running thread's stable, boxed context.
	pub(crate) unsafe fn run(context: NonNull<Self>) -> ! {
		loop {
			let exit = unsafe { (&mut *context.as_ptr()).restore() };
			match exit {
				Ok(exit)
					if matches!(
						exit.reason,
						crate::kernel::arch::VirtualMachineExitReason::PreemptionTimer
							| crate::kernel::arch::VirtualMachineExitReason::ExternalInterrupt
							| crate::kernel::arch::VirtualMachineExitReason::Vmcall
					) =>
				{
					let _switched = super::ThreadManager::switch_current(super::ThreadState::Ready);
				}
				Ok(_) | Err(_) => {
					super::ThreadManager::exit_current(core::ptr::null_mut());
				}
			}
		}
	}
}

#[cfg(feature = "virtualization")]
impl Drop for VcpuContext {
	fn drop(&mut self) {
		let mut retain_guest_memory = false;
		for device in &mut self.passthrough {
			if !device.detach() {
				retain_guest_memory = true;
			}
		}
		if retain_guest_memory {
			crate::kernel::arch::VirtualizationArchitecture::retain_guest_memory(
				&mut self.guest,
			);
		}
	}
}

pub(crate) struct UserAddressSpace;

impl UserAddressSpace {
	pub(crate) const fn new() -> Self {
		Self
	}

	pub(crate) fn allocate_stack(&mut self, requested_length: usize) -> Result<(usize, usize), Error> {
		if requested_length == 0 {
			return Err(Error::NoExec);
		}
		let length = align_up(requested_length, crate::kernel::memory::r#virtual::PAGE_SIZE)
			.ok_or(Error::Overflow)?;
		let address = crate::kernel::memory::r#virtual::reserve_user(
			length,
			crate::kernel::memory::r#virtual::PAGE_SIZE,
			2,
		)
		.ok_or(Error::NoMem)?;
		if self.zero(address, length).is_err() {
			self.release(address, length);
			return Err(Error::NoMem);
		}
		let stack_pointer = address.checked_add(length).ok_or(Error::Overflow)?;
		Ok((address, stack_pointer))
	}

	/// Transfer control to a sealed executable in this user VM arena.
	///
	/// # Safety
	/// The caller must have loaded all dependencies and arranged process state expected by the image.
	pub(crate) unsafe fn enter(&self, object: &LoadedObject, stack_pointer: usize) -> ! {
		if object.kind() != ElfKind::Executable || !object.is_sealed() {
			crate::kernel::arch::CpuArchitecture::halt();
		}
		let Some(entry) = object.entry() else {
			crate::kernel::arch::CpuArchitecture::halt();
		};
		let Some(stack_address) = stack_pointer.checked_sub(1) else {
			crate::kernel::arch::CpuArchitecture::halt();
		};
		if !crate::kernel::memory::r#virtual::user_range_executable(entry, 1)
			|| !crate::kernel::memory::r#virtual::user_range_accessible(stack_address, 1, true)
		{
			crate::kernel::arch::CpuArchitecture::halt();
		}
		unsafe { crate::kernel::arch::ThreadArchitecture::enter_user(entry, stack_pointer) }
	}

	fn reserved_user_range(&self, address: usize, length: usize) -> bool {
		crate::kernel::memory::r#virtual::user_range_reserved(address, length)
	}
}

impl AddressSpace for UserAddressSpace {
	fn reserve(
		&mut self,
		preferred_address: Option<usize>,
		length: usize,
		alignment: usize,
		alignment_offset: usize,
		fixed: bool,
	) -> Result<usize, Error> {
		const PAGE_SIZE: usize = crate::kernel::memory::r#virtual::PAGE_SIZE;
		if length == 0
			|| alignment < PAGE_SIZE
			|| !alignment.is_power_of_two()
			|| alignment_offset >= alignment
			|| alignment_offset % PAGE_SIZE != 0
		{
			return Err(Error::NoExec);
		}
		let length = align_up(length, PAGE_SIZE).ok_or(Error::Overflow)?;
		if fixed {
			let address = preferred_address.ok_or(Error::NoMem)?;
			if address % PAGE_SIZE != 0 || address % alignment != alignment_offset {
				return Err(Error::NoMem);
			}
			return crate::kernel::memory::r#virtual::reserve_user_with_hint(
				Some(address),
				length,
				PAGE_SIZE,
				2,
			)
			.ok_or(Error::NoMem);
		}

		if let Some(address) = preferred_address {
			if address % PAGE_SIZE == 0 && address % alignment == alignment_offset {
				if let Some(reserved) = crate::kernel::memory::r#virtual::reserve_user_with_hint(
					Some(address),
					length,
					PAGE_SIZE,
					2,
				) {
					return Ok(reserved);
				}
			}
		}

		let extra = alignment.checked_sub(PAGE_SIZE).ok_or(Error::Overflow)?;
		let reservation_length = length.checked_add(extra).ok_or(Error::Overflow)?;
		let reservation = crate::kernel::memory::r#virtual::reserve_user(
			reservation_length,
			PAGE_SIZE,
			2,
		)
		.ok_or(Error::NoMem)?;
		let remainder = reservation % alignment;
		let adjustment = if remainder <= alignment_offset {
			alignment_offset - remainder
		} else {
			alignment - (remainder - alignment_offset)
		};
		let address = reservation
			.checked_add(adjustment)
			.ok_or(Error::Overflow)?;
		let prefix_length = address - reservation;
		let suffix_start = address.checked_add(length).ok_or(Error::Overflow)?;
		let suffix_length = reservation_length
			.checked_sub(prefix_length)
			.and_then(|remaining| remaining.checked_sub(length))
			.ok_or(Error::Overflow)?;
		if prefix_length != 0
			&& !crate::kernel::memory::r#virtual::release_range(reservation, prefix_length)
		{
			let _released = crate::kernel::memory::r#virtual::release(reservation, reservation_length);
			return Err(Error::NoMem);
		}
		if suffix_length != 0
			&& !crate::kernel::memory::r#virtual::release_range(suffix_start, suffix_length)
		{
			let _released = crate::kernel::memory::r#virtual::release(address, length + suffix_length);
			return Err(Error::NoMem);
		}
		Ok(address)
	}

	fn zero(&mut self, address: usize, length: usize) -> Result<(), Error> {
		if !self.reserved_user_range(address, length) {
			return Err(Error::NoMem);
		}
		unsafe { core::ptr::write_bytes(address as *mut u8, 0, length) };
		Ok(())
	}

	fn read(&self, address: usize, buffer: &mut [u8]) -> Result<(), Error> {
		if !crate::kernel::memory::r#virtual::user_range_accessible(address, buffer.len(), false) {
			return Err(Error::NoMem);
		}
		unsafe {
			core::ptr::copy_nonoverlapping(address as *const u8, buffer.as_mut_ptr(), buffer.len())
		};
		Ok(())
	}

	fn write(&mut self, address: usize, buffer: &[u8]) -> Result<(), Error> {
		if !self.reserved_user_range(address, buffer.len())
			|| !crate::kernel::memory::r#virtual::user_range_accessible(
				address,
				buffer.len(),
				true,
			)
		{
			return Err(Error::NoMem);
		}
		unsafe {
			core::ptr::copy_nonoverlapping(buffer.as_ptr(), address as *mut u8, buffer.len())
		};
		Ok(())
	}

	fn protect(
		&mut self,
		address: usize,
		length: usize,
		permissions: Permissions,
	) -> Result<(), Error> {
		let permissions = elf_permissions_to_vm(permissions);
		if crate::kernel::memory::r#virtual::set_user_permissions(address, length, permissions) {
			Ok(())
		} else {
			Err(Error::NoMem)
		}
	}

	fn release(&mut self, address: usize, length: usize) {
		let _released = crate::kernel::memory::r#virtual::release(address, length);
	}
}

fn elf_permissions_to_vm(permissions: Permissions) -> i32 {
	let bits = permissions.bits();
	let writable = bits & Permissions::WRITE.bits() != 0;
	let executable = bits & Permissions::EXECUTE.bits() != 0;
	match (writable, executable) {
		(false, false) if bits == 0 => 0,
		(false, false) => 1,
		(true, false) => 2,
		(false, true) => 4,
		(true, true) => 3,
	}
}

