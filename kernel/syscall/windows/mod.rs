/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[path = "fn/mod.rs"]
mod r#fn;
pub(crate) mod error;

use super::SyscallFrame;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

pub(crate) fn dispatch_call(frame: SyscallFrame) -> Result<u32, Error> {
	match frame.number {
		r#fn::delay_execution::DelayExecution::ID => {
			r#fn::delay_execution::DelayExecution::handle(frame)
		}
		r#fn::query_performance_counter::QueryPerformanceCounter::ID => {
			r#fn::query_performance_counter::QueryPerformanceCounter::handle(frame)
		}
		r#fn::terminate_process::TerminateProcess::ID => {
			r#fn::terminate_process::TerminateProcess::handle(frame)
		}
		r#fn::yield_execution::YieldExecution::ID => {
			r#fn::yield_execution::YieldExecution::handle(frame)
		}
		_ => Err(Error::NoSys),
	}
}

pub(crate) fn dispatch_call_with_user_stack_pointer(
	frame: SyscallFrame,
	user_rsp: usize,
) -> Result<u32, Error> {
	match frame.number {
		r#fn::write_file::WriteFile::ID => {
			r#fn::write_file::WriteFile::handle(frame, user_rsp)
		}
		_ => dispatch_call(frame),
	}
}

pub(crate) fn dispatch(frame: &mut SyscallFrame) {
	ThreadManager::begin_syscall_accounting();
	frame.errno = 0;
	frame.result = match dispatch_call(*frame) {
		Ok(status) => i64::from(status as i32),
		Err(error) => i64::from(error::ntstatus(error) as i32),
	};
	ThreadManager::end_syscall_accounting();
}

#[cfg(target_arch = "x86_64")]
pub(crate) fn dispatch_with_user_stack_pointer(frame: &mut SyscallFrame, user_rsp: usize) {
	ThreadManager::begin_syscall_accounting();
	frame.errno = 0;
	frame.result = match dispatch_call_with_user_stack_pointer(*frame, user_rsp) {
		Ok(status) => i64::from(status as i32),
		Err(error) => i64::from(error::ntstatus(error) as i32),
	};
	ThreadManager::end_syscall_accounting();
}

#[cfg(test)]
#[path = "../../../tests/kernel/syscall/windows/dispatch.rs"]
mod tests;