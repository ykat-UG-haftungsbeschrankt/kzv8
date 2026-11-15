/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[allow(dead_code)]
pub(crate) mod darwin;
pub(crate) mod linux;
#[allow(dead_code)]
pub(crate) mod windows;

pub(crate) use linux::SyscallFrame;

pub(crate) fn dispatch(frame: &mut SyscallFrame) {
	if crate::kernel::scheduler::thread::ThreadManager::current_process()
		.is_ok_and(|process| process.syscall_abi() == crate::kernel::process::SyscallAbi::WindowsNt)
	{
		windows::dispatch(frame);
	} else if darwin::has_syscall_class(frame.number) {
		darwin::dispatch(frame);
	} else {
		linux::dispatch(frame);
	}
}

#[cfg(target_arch = "x86_64")]
pub(crate) fn dispatch_with_user_stack_pointer(frame: &mut SyscallFrame, user_rsp: usize) {
	if crate::kernel::scheduler::thread::ThreadManager::current_process()
		.is_ok_and(|process| process.syscall_abi() == crate::kernel::process::SyscallAbi::WindowsNt)
	{
		windows::dispatch_with_user_stack_pointer(frame, user_rsp);
	} else {
		dispatch(frame);
	}
}

#[cfg(target_arch = "x86_64")]
pub(crate) fn dispatch_with_user_registers(
	frame: &mut SyscallFrame,
	registers: crate::kernel::arch::UserRegisterState,
) {
	if crate::kernel::scheduler::thread::ThreadManager::current_process()
		.is_ok_and(|process| process.syscall_abi() == crate::kernel::process::SyscallAbi::WindowsNt)
		|| darwin::has_syscall_class(frame.number)
	{
		dispatch(frame);
	} else {
		linux::dispatch_with_user_registers(frame, registers);
	}
}
