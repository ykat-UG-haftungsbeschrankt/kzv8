/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ops::Range;
use core::sync::atomic::{AtomicU16, Ordering};

use crate::kernel::dl::{AddressSpace, ElfKind, Error, LoadedObject, Permissions};
use crate::kernel::arch::traits::{
	CpuArchitecture as _, InterruptArchitecture as _, MemoryArchitecture as _,
	ThreadArchitecture as _,
};
use crate::kernel::lib::alignment::align_up;
use crate::kernel::memory::physical::PhysicalAllocator;
use crate::kernel::scheduler::sync::IrqSpinLock;
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
	pub(crate) unsafe fn run(context: core::ptr::NonNull<Self>) -> ! {
		loop {
			let exit = unsafe { (&mut *context.as_ptr()).restore() };
			match exit {
				#[cfg(target_arch = "x86_64")]
				Ok(exit)
					if exit.reason
						== crate::kernel::arch::VirtualMachineExitReason::ExceptionOrNmi
						&& exit.interrupt_info & (1 << 31) != 0
						&& (exit.interrupt_info >> 8) & 0x7 == 2
						&& exit.interrupt_info & 0xff == 2
						&& crate::kernel::arch::handle_tlb_shootdown_nmi() => {}
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

const USER_VM_PAGE_SIZE: usize = 4096;
const VM_PERMISSION_MASK: u16 = 0x07;
const VM_USER: u16 = 1 << 6;
const VM_RESERVED: u16 = 1 << 7;
const VM_OWNED: u16 = 1 << 8;
const VM_BORROWED: u16 = 1 << 9;

struct UserArea {
	range: Range<usize>,
	page_states: Vec<AtomicU16>,
}

struct UserAreas {
	areas: Vec<UserArea>,
}

impl UserAreas {
	const fn new() -> Self {
		Self { areas: Vec::new() }
	}

	fn area_index(&self, address: usize) -> Option<usize> {
		let index = self
			.areas
			.partition_point(|area| area.range.start <= address)
			.checked_sub(1)?;
		(self.areas[index].range.end > address).then_some(index)
	}

	fn reserve(
		&mut self,
		bounds: Range<usize>,
		hint: Option<usize>,
		length: usize,
		alignment: usize,
		permissions: i32,
	) -> Option<usize> {
		if length == 0
			|| alignment == 0
			|| !alignment.is_power_of_two()
			|| !(0..=5).contains(&permissions)
		{
			return None;
		}
		let length = align_up(length, USER_VM_PAGE_SIZE)?;
		let alignment = alignment.max(USER_VM_PAGE_SIZE);
		let range = if let Some(address) = hint {
			let end = address.checked_add(length)?;
			let insertion = self
				.areas
				.partition_point(|area| area.range.start < address);
			if address < bounds.start || address % alignment != 0 || end > bounds.end {
				return None;
			}
			if insertion > 0 && self.areas[insertion - 1].range.end > address {
				return None;
			}
			if insertion < self.areas.len() && self.areas[insertion].range.start < end {
				return None;
			}
			address..end
		} else {
			let mut cursor = bounds.start;
			let mut fit = None;
			for area in &self.areas {
				let start = align_up(cursor, alignment)?;
				let end = start.checked_add(length)?;
				if end <= area.range.start {
					fit = Some(start..end);
					break;
				}
				cursor = cursor.max(area.range.end);
			}
			if let Some(fit) = fit {
				fit
			} else {
				let start = align_up(cursor, alignment)?;
				let end = start.checked_add(length)?;
				if end > bounds.end {
					return None;
				}
				start..end
			}
		};

		let page_count = length / USER_VM_PAGE_SIZE;
		let state_permissions = if permissions == 5 {
			0
		} else {
			permissions as u16
		};
		let state = VM_RESERVED | VM_USER | state_permissions;
		let mut page_states = Vec::new();
		page_states.try_reserve_exact(page_count).ok()?;
		page_states.resize_with(page_count, || AtomicU16::new(state));
		self.areas.try_reserve(1).ok()?;
		let insertion = self
			.areas
			.partition_point(|area| area.range.start < range.start);
		self.areas.insert(
			insertion,
			UserArea {
				range: range.clone(),
				page_states,
			},
		);
		Some(range.start)
	}

	fn is_reserved(&self, range: &Range<usize>) -> bool {
		if range.start == range.end {
			return true;
		}
		let Some(area_index) = self.area_index(range.start) else {
			return false;
		};
		let area = &self.areas[area_index];
		if range.end > area.range.end {
			return false;
		}
		let first_page = (range.start - area.range.start) / USER_VM_PAGE_SIZE;
		let page_count = (range.end - range.start) / USER_VM_PAGE_SIZE;
		(first_page..first_page + page_count).all(|page| {
			area.page_states.get(page).is_some_and(|state| {
				state.load(Ordering::Acquire) & (VM_RESERVED | VM_USER)
					== (VM_RESERVED | VM_USER)
			})
		})
	}
}

struct UserAddressSpaceInner {
	root: usize,
	base: usize,
	length: usize,
	areas: IrqSpinLock<UserAreas>,
	#[cfg(test)]
	test_stub: bool,
}

impl Drop for UserAddressSpaceInner {
	fn drop(&mut self) {
		#[cfg(test)]
		if self.test_stub {
			return;
		}
		let interrupt_state = crate::kernel::arch::InterruptArchitecture::disable_interrupts();
		let previous_root = crate::kernel::arch::MemoryArchitecture::current_address_space();
		if previous_root == self.root
			|| !crate::kernel::arch::MemoryArchitecture::activate_address_space(self.root)
		{
			crate::kernel::arch::CpuArchitecture::halt();
		}
		{
			let areas = self.areas.lock();
			for area in &areas.areas {
				for (page, state) in area.page_states.iter().enumerate() {
					let address = area.range.start + page * USER_VM_PAGE_SIZE;
					if let Some(physical_address) =
						crate::kernel::arch::MemoryArchitecture::unmap_page(address)
					{
						if state.load(Ordering::Acquire) & VM_BORROWED == 0
							&& !PhysicalAllocator::deallocate_contiguous(
								physical_address,
								USER_VM_PAGE_SIZE,
							)
						{
							crate::kernel::arch::CpuArchitecture::halt();
						}
					}
				}
			}
		}
		if !crate::kernel::arch::MemoryArchitecture::activate_address_space(previous_root)
			|| !crate::kernel::arch::MemoryArchitecture::destroy_address_space(
				self.root,
				self.base,
				self.length,
			)
		{
			crate::kernel::arch::CpuArchitecture::halt();
		}
		crate::kernel::arch::InterruptArchitecture::restore_interrupts(interrupt_state);
	}
}

#[derive(Clone)]
pub(crate) struct UserAddressSpace {
	inner: Arc<UserAddressSpaceInner>,
}

impl UserAddressSpace {
	pub(crate) fn new() -> Result<Self, Error> {
		let (root, base, length) = crate::kernel::arch::MemoryArchitecture::create_address_space()
			.ok_or(Error::NoMem)?;
		Ok(Self {
			inner: Arc::new(UserAddressSpaceInner {
				root,
				base,
				length,
				areas: IrqSpinLock::new(UserAreas::new()),
				#[cfg(test)]
				test_stub: false,
			}),
		})
	}

	#[cfg(test)]
	pub(crate) fn new_test_stub() -> Self {
		Self {
			inner: Arc::new(UserAddressSpaceInner {
				root: 0,
				base: 0,
				length: 0,
				areas: IrqSpinLock::new(UserAreas::new()),
				test_stub: true,
			}),
		}
	}

	pub(crate) fn root(&self) -> usize {
		self.inner.root
	}

	fn bounds(&self) -> Option<Range<usize>> {
		Some(self.inner.base..self.inner.base.checked_add(self.inner.length)?)
	}

	fn rounded_range(&self, address: usize, length: usize) -> Option<Range<usize>> {
		if length == 0 {
			return Some(address..address);
		}
		let end = address.checked_add(length)?;
		let start = address & !(USER_VM_PAGE_SIZE - 1);
		let end = align_up(end, USER_VM_PAGE_SIZE)?;
		let bounds = self.bounds()?;
		(start >= bounds.start && end <= bounds.end && start < end).then_some(start..end)
	}

	fn reservation_range(&self, address: usize, length: usize) -> Option<Range<usize>> {
		if length == 0 || address % USER_VM_PAGE_SIZE != 0 {
			return None;
		}
		self.rounded_range(address, length)
	}

	fn activate<T>(&self, operation: impl FnOnce() -> T) -> Option<T> {
		let interrupt_state = crate::kernel::arch::InterruptArchitecture::disable_interrupts();
		let previous_root = crate::kernel::arch::MemoryArchitecture::current_address_space();
		if previous_root != self.inner.root
			&& !crate::kernel::arch::MemoryArchitecture::activate_address_space(self.inner.root)
		{
			crate::kernel::arch::InterruptArchitecture::restore_interrupts(interrupt_state);
			return None;
		}
		let result = operation();
		if previous_root != self.inner.root
			&& !crate::kernel::arch::MemoryArchitecture::activate_address_space(previous_root)
		{
			crate::kernel::arch::CpuArchitecture::halt();
		}
		crate::kernel::arch::InterruptArchitecture::restore_interrupts(interrupt_state);
		Some(result)
	}

	fn reserve_user(
		&self,
		hint: Option<usize>,
		length: usize,
		alignment: usize,
		permissions: i32,
	) -> Option<usize> {
		self.inner
			.areas
			.lock()
			.reserve(self.bounds()?, hint, length, alignment, permissions)
	}

	pub(crate) fn reserve_user_mapping(
		&self,
		hint: Option<usize>,
		length: usize,
		permissions: i32,
		fixed: bool,
	) -> Option<usize> {
		if length == 0 || !(0..=4).contains(&permissions) {
			return None;
		}
		if let Some(address) = hint {
			if address % USER_VM_PAGE_SIZE != 0 {
				return None;
			}
			if let Some(reserved) = self.reserve_user(
				Some(address),
				length,
				USER_VM_PAGE_SIZE,
				permissions,
			) {
				return Some(reserved);
			}
			if fixed {
				return None;
			}
		} else if fixed {
			return None;
		}
		self.reserve_user(None, length, USER_VM_PAGE_SIZE, permissions)
	}

	pub(crate) fn reserve_user_mapping_limited(
		&self,
		hint: Option<usize>,
		length: usize,
		permissions: i32,
		limit: u64,
	) -> Option<usize> {
		if length == 0 || !(0..=4).contains(&permissions) {
			return None;
		}
		let rounded_length = align_up(length, USER_VM_PAGE_SIZE)?;
		let mut areas = self.inner.areas.lock();
		let reserved = areas.areas.iter().fold(0usize, |total, area| {
			total.saturating_add(area.range.end - area.range.start)
		});
		let requested = u64::try_from(rounded_length).ok()?;
		if u64::try_from(reserved).ok()?.saturating_add(requested) > limit {
			return None;
		}
		if let Some(address) = hint {
			if address % USER_VM_PAGE_SIZE != 0 {
				return None;
			}
			if let Some(reserved_address) = areas.reserve(
				self.bounds()?,
				Some(address),
				rounded_length,
				USER_VM_PAGE_SIZE,
				permissions,
			) {
				return Some(reserved_address);
			}
		}
		areas.reserve(
			self.bounds()?,
			None,
			rounded_length,
			USER_VM_PAGE_SIZE,
			permissions,
		)
	}

	pub(crate) fn user_range_reserved(&self, address: usize, length: usize) -> bool {
		if length == 0 {
			return true;
		}
		let Some(range) = self.rounded_range(address, length) else {
			return false;
		};
		self.inner.areas.lock().is_reserved(&range)
	}

	pub(crate) fn user_range_accessible(&self, address: usize, length: usize, write: bool) -> bool {
		self.activate(|| {
			(self.user_range_reserved(address, length)
				|| length == 0)
				&& crate::kernel::arch::MemoryArchitecture::user_range_accessible(
					address, length, write,
				)
		})
		.unwrap_or(false)
	}

	pub(crate) fn user_range_executable(&self, address: usize, length: usize) -> bool {
		self.activate(|| {
			(self.user_range_reserved(address, length) || length == 0)
				&& crate::kernel::arch::MemoryArchitecture::user_range_executable(address, length)
		})
		.unwrap_or(false)
	}

	pub(crate) fn release_user_mapping(&self, address: usize, length: usize) -> bool {
		self.release_range(address, length, false)
	}

	fn release_range(&self, address: usize, length: usize, exact: bool) -> bool {
		let Some(range) = self.reservation_range(address, length) else {
			return false;
		};
		self.activate(|| {
			let mut areas = self.inner.areas.lock();
			let Some(area_index) = areas.area_index(range.start) else {
				return false;
			};
			let (area_range, first_page, end_page, prefix_pages, suffix_pages) = {
				let area = &areas.areas[area_index];
				let area_range = area.range.clone();
				if range.end > area_range.end || (exact && range != area_range) {
					return false;
				}
				let first_page = (range.start - area_range.start) / USER_VM_PAGE_SIZE;
				let end_page = (range.end - area_range.start) / USER_VM_PAGE_SIZE;
				if (first_page..end_page).any(|page| {
					area.page_states[page].load(Ordering::Acquire) & (VM_RESERVED | VM_USER)
						!= (VM_RESERVED | VM_USER)
				}) {
					return false;
				}
				(
					area_range,
					first_page,
					end_page,
					first_page,
					area.page_states.len() - end_page,
				)
			};
			if prefix_pages != 0
				&& suffix_pages != 0
				&& areas.areas.try_reserve(1).is_err()
			{
				return false;
			}
			let mut area = areas.areas.remove(area_index);
			let suffix_states = area.page_states.split_off(end_page);
			let released_states = area.page_states.split_off(first_page);
			let mut released = true;
			for (offset, state) in released_states.iter().enumerate() {
				let virtual_address = range.start + offset * USER_VM_PAGE_SIZE;
				let state = state.load(Ordering::Acquire);
				if let Some(physical_address) =
					crate::kernel::arch::MemoryArchitecture::unmap_page(virtual_address)
				{
					if state & VM_BORROWED == 0
						&& !PhysicalAllocator::deallocate_contiguous(
							physical_address,
							USER_VM_PAGE_SIZE,
						)
					{
						released = false;
					}
				}
			}
			match (prefix_pages != 0, suffix_pages != 0) {
				(true, true) => {
					area.range = area_range.start..range.start;
					areas.areas.insert(area_index, area);
					areas.areas.insert(
						area_index + 1,
						UserArea {
							range: range.end..area_range.end,
							page_states: suffix_states,
						},
					);
				}
				(true, false) => {
					area.range = area_range.start..range.start;
					areas.areas.insert(area_index, area);
				}
				(false, true) => {
					areas.areas.insert(
						area_index,
						UserArea {
							range: range.end..area_range.end,
							page_states: suffix_states,
						},
					);
				}
				(false, false) => {}
			}
			released
		})
		.unwrap_or(false)
	}

	fn set_permissions(&self, address: usize, length: usize, permissions: i32) -> bool {
		if !(0..=4).contains(&permissions) {
			return false;
		}
		let Some(range) = self.reservation_range(address, length) else {
			return false;
		};
		self.activate(|| {
			let areas = self.inner.areas.lock();
			let Some(area_index) = areas.area_index(range.start) else {
				return false;
			};
			let area = &areas.areas[area_index];
			if range.end > area.range.end {
				return false;
			}
			let first_page = (range.start - area.range.start) / USER_VM_PAGE_SIZE;
			let end_page = (range.end - area.range.start) / USER_VM_PAGE_SIZE;
			if (first_page..end_page).any(|page| {
				area.page_states[page].load(Ordering::Acquire) & (VM_RESERVED | VM_USER)
					!= (VM_RESERVED | VM_USER)
			}) {
				return false;
			}
			for page in first_page..end_page {
				let state = area.page_states[page].load(Ordering::Acquire);
				let virtual_address = range.start + (page - first_page) * USER_VM_PAGE_SIZE;
				if crate::kernel::arch::MemoryArchitecture::page_has_backing(virtual_address)
					&& !crate::kernel::arch::MemoryArchitecture::set_user_page_permissions(
						virtual_address,
						USER_VM_PAGE_SIZE,
						permissions,
					)
				{
					return false;
				}
				area.page_states[page].store(
					VM_RESERVED
						| (state & (VM_USER | VM_OWNED | VM_BORROWED))
						| permissions as u16,
					Ordering::Release,
				);
			}
			true
		})
		.unwrap_or(false)
	}

	pub(crate) fn protect_user_mapping(
		&self,
		address: usize,
		length: usize,
		permissions: i32,
	) -> bool {
		self.user_range_reserved(address, length) && self.set_permissions(address, length, permissions)
	}

	pub(crate) fn map_user_borrowed_page(
		&self,
		virtual_address: usize,
		physical_address: usize,
		permissions: i32,
	) -> bool {
		if virtual_address % USER_VM_PAGE_SIZE != 0
			|| physical_address % USER_VM_PAGE_SIZE != 0
			|| !(0..=4).contains(&permissions)
		{
			return false;
		}
		let Some(range) = self.reservation_range(virtual_address, USER_VM_PAGE_SIZE) else {
			return false;
		};
		self.activate(|| {
			let areas = self.inner.areas.lock();
			let Some(area_index) = areas.area_index(range.start) else {
				return false;
			};
			let area = &areas.areas[area_index];
			let page = (range.start - area.range.start) / USER_VM_PAGE_SIZE;
			let Some(state) = area
				.page_states
				.get(page)
				.map(|state| state.load(Ordering::Acquire))
			else {
				return false;
			};
			if state & (VM_RESERVED | VM_USER) != (VM_RESERVED | VM_USER)
				|| state & (VM_OWNED | VM_BORROWED) != 0
				|| !crate::kernel::arch::MemoryArchitecture::map_user_page(
					virtual_address,
					physical_address,
					permissions,
				)
			{
				return false;
			}
			area.page_states[page].store(
				(state & !VM_PERMISSION_MASK) | permissions as u16 | VM_BORROWED,
				Ordering::Release,
			);
			true
		})
		.unwrap_or(false)
	}

	pub(crate) fn resolve_page_fault(
		&self,
		address: usize,
		access: crate::kernel::memory::r#virtual::FaultAccess,
	) -> bool {
		let page_address = address & !(USER_VM_PAGE_SIZE - 1);
		let Some(range) = self.reservation_range(page_address, USER_VM_PAGE_SIZE) else {
			return false;
		};
		self.activate(|| {
			let areas = self.inner.areas.lock();
			let Some(area_index) = areas.area_index(range.start) else {
				return false;
			};
			let area = &areas.areas[area_index];
			let page = (range.start - area.range.start) / USER_VM_PAGE_SIZE;
			let Some(state) = area
				.page_states
				.get(page)
				.map(|state| state.load(Ordering::Acquire))
			else {
				return false;
			};
			if state & (VM_RESERVED | VM_USER) != (VM_RESERVED | VM_USER) {
				return false;
			}
			let permissions = (state & VM_PERMISSION_MASK) as i32;
			let allowed = match access {
				crate::kernel::memory::r#virtual::FaultAccess::Read => (1..=4).contains(&permissions),
				crate::kernel::memory::r#virtual::FaultAccess::Write => matches!(permissions, 2 | 3),
				crate::kernel::memory::r#virtual::FaultAccess::Execute => matches!(permissions, 3 | 4),
			};
			if !allowed {
				return false;
			}
			if crate::kernel::arch::MemoryArchitecture::page_is_mapped(page_address) {
				return true;
			}
			let Some(frame) = PhysicalAllocator::allocate() else {
				return false;
			};
			if !crate::kernel::arch::MemoryArchitecture::map_user_page(
				page_address,
				frame.address(),
				2,
			) {
				let _released = PhysicalAllocator::deallocate(frame);
				return false;
			}
			unsafe { core::ptr::write_bytes(page_address as *mut u8, 0, USER_VM_PAGE_SIZE) };
			if permissions == 2
				|| crate::kernel::arch::MemoryArchitecture::set_user_page_permissions(
					page_address,
					USER_VM_PAGE_SIZE,
					permissions,
				)
			{
				area.page_states[page].fetch_or(VM_OWNED, Ordering::Release);
				return true;
			}
			let _unmapped = crate::kernel::arch::MemoryArchitecture::unmap_page(page_address);
			let _released = PhysicalAllocator::deallocate(frame);
			false
		})
		.unwrap_or(false)
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
		if !self.user_range_executable(entry, 1)
			|| !self.user_range_accessible(stack_address, 1, true)
			|| crate::kernel::arch::MemoryArchitecture::current_address_space() != self.inner.root
		{
			crate::kernel::arch::CpuArchitecture::halt();
		}
		unsafe { crate::kernel::arch::ThreadArchitecture::enter_user(entry, stack_pointer) }
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
		if length == 0
			|| alignment < USER_VM_PAGE_SIZE
			|| !alignment.is_power_of_two()
			|| alignment_offset >= alignment
			|| alignment_offset % USER_VM_PAGE_SIZE != 0
		{
			return Err(Error::NoExec);
		}
		let length = align_up(length, USER_VM_PAGE_SIZE).ok_or(Error::Overflow)?;
		if fixed {
			let address = preferred_address.ok_or(Error::NoMem)?;
			if address % USER_VM_PAGE_SIZE != 0 || address % alignment != alignment_offset {
				return Err(Error::NoMem);
			}
			return self
				.reserve_user(Some(address), length, USER_VM_PAGE_SIZE, 2)
				.ok_or(Error::NoMem);
		}

		if let Some(address) = preferred_address {
			if address % USER_VM_PAGE_SIZE == 0 && address % alignment == alignment_offset {
				if let Some(reserved) = self.reserve_user(
					Some(address),
					length,
					USER_VM_PAGE_SIZE,
					2,
				) {
					return Ok(reserved);
				}
			}
		}

		let extra = alignment
			.checked_sub(USER_VM_PAGE_SIZE)
			.ok_or(Error::Overflow)?;
		let reservation_length = length.checked_add(extra).ok_or(Error::Overflow)?;
		let reservation = self
			.reserve_user(None, reservation_length, USER_VM_PAGE_SIZE, 2)
			.ok_or(Error::NoMem)?;
		let remainder = reservation % alignment;
		let adjustment = if remainder <= alignment_offset {
			alignment_offset - remainder
		} else {
			alignment - (remainder - alignment_offset)
		};
		let address = reservation.checked_add(adjustment).ok_or(Error::Overflow)?;
		let prefix_length = address - reservation;
		let suffix_start = address.checked_add(length).ok_or(Error::Overflow)?;
		let suffix_length = reservation_length
			.checked_sub(prefix_length)
			.and_then(|remaining| remaining.checked_sub(length))
			.ok_or(Error::Overflow)?;
		if prefix_length != 0 && !self.release_range(reservation, prefix_length, false) {
			let _released = self.release_range(reservation, reservation_length, true);
			return Err(Error::NoMem);
		}
		if suffix_length != 0 && !self.release_range(suffix_start, suffix_length, false) {
			let _released = self.release_range(address, length + suffix_length, false);
			return Err(Error::NoMem);
		}
		Ok(address)
	}

	fn zero(&mut self, address: usize, length: usize) -> Result<(), Error> {
		if length == 0 {
			return Ok(());
		}
		if !self.user_range_reserved(address, length) {
			return Err(Error::NoMem);
		}
		let range = self.rounded_range(address, length).ok_or(Error::NoMem)?;
		for page in (range.start..range.end).step_by(USER_VM_PAGE_SIZE) {
			if !self.resolve_page_fault(
				page,
				crate::kernel::memory::r#virtual::FaultAccess::Write,
			) {
				return Err(Error::NoMem);
			}
		}
		let copied = self.activate(|| unsafe {
			core::ptr::write_bytes(address as *mut u8, 0, length);
			true
		});
		if copied == Some(true) {
			Ok(())
		} else {
			Err(Error::NoMem)
		}
	}

	fn read(&self, address: usize, buffer: &mut [u8]) -> Result<(), Error> {
		if buffer.is_empty() {
			return Ok(());
		}
		let copied = self.activate(|| {
			if !self.user_range_reserved(address, buffer.len())
				|| !crate::kernel::arch::MemoryArchitecture::user_range_accessible(
					address,
					buffer.len(),
					false,
				)
			{
				return false;
			}
			unsafe {
				core::ptr::copy_nonoverlapping(
					address as *const u8,
					buffer.as_mut_ptr(),
					buffer.len(),
				)
			};
			true
		});
		if copied == Some(true) {
			Ok(())
		} else {
			Err(Error::NoMem)
		}
	}

	fn write(&mut self, address: usize, buffer: &[u8]) -> Result<(), Error> {
		if buffer.is_empty() {
			return Ok(());
		}
		let copied = self.activate(|| {
			if !self.user_range_reserved(address, buffer.len())
				|| !crate::kernel::arch::MemoryArchitecture::user_range_accessible(
					address,
					buffer.len(),
					true,
				)
			{
				return false;
			}
			unsafe {
				core::ptr::copy_nonoverlapping(
					buffer.as_ptr(),
					address as *mut u8,
					buffer.len(),
				)
			};
			true
		});
		if copied == Some(true) {
			Ok(())
		} else {
			Err(Error::NoMem)
		}
	}

	fn protect(
		&mut self,
		address: usize,
		length: usize,
		permissions: Permissions,
	) -> Result<(), Error> {
		if self.set_permissions(address, length, elf_permissions_to_vm(permissions)) {
			Ok(())
		} else {
			Err(Error::NoMem)
		}
	}

	fn release(&mut self, address: usize, length: usize) {
		let _released = self.release_range(address, length, true);
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

