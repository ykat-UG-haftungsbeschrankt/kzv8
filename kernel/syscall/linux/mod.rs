/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[path = "fn/mod.rs"]
mod r#fn;
mod error;

use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SyscallFrame {
    pub(crate) number: u64,
    pub(crate) arguments: [u64; 6],
    pub(crate) windows_arguments: [u64; 4],
    pub(crate) result: i64,
    pub(crate) errno: u32,
}

pub(crate) fn dispatch_call(frame: SyscallFrame) -> Result<i64, Error> {
    match frame.number {
        #[cfg(target_arch = "x86_64")]
        r#fn::arch_prctl::ArchPrctl::ID => r#fn::arch_prctl::ArchPrctl::handle(frame),
        r#fn::exit::Exit::ID => r#fn::exit::Exit::handle(frame),
        r#fn::exit_group::ExitGroup::ID => r#fn::exit_group::ExitGroup::handle(frame),
        #[cfg(target_arch = "x86_64")]
        r#fn::execve::Execve::ID => r#fn::execve::Execve::handle(frame),
        r#fn::getpid::GetPid::ID => r#fn::getpid::GetPid::handle(frame),
        r#fn::process_group::GetPgid::ID => r#fn::process_group::GetPgid::handle(frame),
        r#fn::process_group::GetSid::ID => r#fn::process_group::GetSid::handle(frame),
        r#fn::process_group::SetPgid::ID => r#fn::process_group::SetPgid::handle(frame),
        r#fn::process_group::SetSid::ID => r#fn::process_group::SetSid::handle(frame),
        r#fn::getppid::GetPpid::ID => r#fn::getppid::GetPpid::handle(frame),
        r#fn::gettid::GetTid::ID => r#fn::gettid::GetTid::handle(frame),
        r#fn::kill::Kill::ID => r#fn::kill::Kill::handle(frame),
        r#fn::links::LinkAt::ID => r#fn::links::LinkAt::handle(frame),
        r#fn::links::SymlinkAt::ID => r#fn::links::SymlinkAt::handle(frame),
        r#fn::sched_yield::SchedYield::ID => r#fn::sched_yield::SchedYield::handle(frame),
        r#fn::sync::Sync::ID => r#fn::sync::Sync::handle(frame),
        r#fn::signal_mask::RtSigprocmask::ID => r#fn::signal_mask::RtSigprocmask::handle(frame),
        r#fn::signal_pending::RtSigpending::ID => r#fn::signal_pending::RtSigpending::handle(frame),
        r#fn::signal_action::RtSigaction::ID => r#fn::signal_action::RtSigaction::handle(frame),
           r#fn::pread64::Pread64::ID => r#fn::pread64::Pread64::handle(frame),
        r#fn::read::Read::ID => r#fn::read::Read::handle(frame),
        #[cfg(target_arch = "x86_64")]
        r#fn::poll::Poll::ID => r#fn::poll::Poll::handle(frame),
        r#fn::pread64::Pread64::ID => r#fn::pread64::Pread64::handle(frame),
        r#fn::futex::Futex::ID => r#fn::futex::Futex::handle(frame),
        r#fn::readlinkat::ReadlinkAt::ID => r#fn::readlinkat::ReadlinkAt::handle(frame),
        r#fn::unlinkat::UnlinkAt::ID => r#fn::unlinkat::UnlinkAt::handle(frame),
        r#fn::write::Write::ID => r#fn::write::Write::handle(frame),
        r#fn::writev::Writev::ID => r#fn::writev::Writev::handle(frame),
        r#fn::close::Close::ID => r#fn::close::Close::handle(frame),
        r#fn::ioctl::Ioctl::ID => r#fn::ioctl::Ioctl::handle(frame),
        r#fn::metadata::Chmod::ID => r#fn::metadata::Chmod::handle(frame),
        r#fn::metadata::Fchmod::ID => r#fn::metadata::Fchmod::handle(frame),
        r#fn::metadata::Chown::ID => r#fn::metadata::Chown::handle(frame),
        r#fn::metadata::Fchown::ID => r#fn::metadata::Fchown::handle(frame),
        r#fn::dup::Dup::ID => r#fn::dup::Dup::handle(frame),
        r#fn::credentials::GetUid::ID => r#fn::credentials::GetUid::handle(frame),
        r#fn::credentials::GetEuid::ID => r#fn::credentials::GetEuid::handle(frame),
        r#fn::credentials::GetGid::ID => r#fn::credentials::GetGid::handle(frame),
        r#fn::credentials::GetEgid::ID => r#fn::credentials::GetEgid::handle(frame),
        r#fn::credentials::GetResUid::ID => r#fn::credentials::GetResUid::handle(frame),
        r#fn::credentials::GetResGid::ID => r#fn::credentials::GetResGid::handle(frame),
        r#fn::credentials::SetUid::ID => r#fn::credentials::SetUid::handle(frame),
        r#fn::credentials::SetGid::ID => r#fn::credentials::SetGid::handle(frame),
        r#fn::credentials::SetReUid::ID => r#fn::credentials::SetReUid::handle(frame),
        r#fn::credentials::SetReGid::ID => r#fn::credentials::SetReGid::handle(frame),
        r#fn::credentials::SetResUid::ID => r#fn::credentials::SetResUid::handle(frame),
        r#fn::credentials::SetResGid::ID => r#fn::credentials::SetResGid::handle(frame),
        r#fn::getgroups::GetGroups::ID => r#fn::getgroups::GetGroups::handle(frame),
        r#fn::getrandom::Getrandom::ID => r#fn::getrandom::Getrandom::handle(frame),
        r#fn::getrusage::Getrusage::ID => r#fn::getrusage::Getrusage::handle(frame),
        r#fn::wait4::Wait4::ID => r#fn::wait4::Wait4::handle(frame),
        r#fn::rlimit::Getrlimit::ID => r#fn::rlimit::Getrlimit::handle(frame),
        r#fn::rlimit::Setrlimit::ID => r#fn::rlimit::Setrlimit::handle(frame),
        r#fn::priority::Getpriority::ID => r#fn::priority::Getpriority::handle(frame),
        r#fn::priority::Setpriority::ID => r#fn::priority::Setpriority::handle(frame),
        r#fn::getgroups::SetGroups::ID => r#fn::getgroups::SetGroups::handle(frame),
        r#fn::umask::Umask::ID => r#fn::umask::Umask::handle(frame),
        r#fn::getdents64::GetDents64::ID => r#fn::getdents64::GetDents64::handle(frame),
        r#fn::stat::Fstat::ID => r#fn::stat::Fstat::handle(frame),
        r#fn::stat::Newfstatat::ID => r#fn::stat::Newfstatat::handle(frame),
        #[cfg(target_arch = "x86_64")]
        r#fn::stat::Stat::ID => r#fn::stat::Stat::handle(frame),
        #[cfg(target_arch = "x86_64")]
        r#fn::stat::Lstat::ID => r#fn::stat::Lstat::handle(frame),
            r#fn::getcwd::GetCwd::ID => r#fn::getcwd::GetCwd::handle(frame),
            r#fn::lseek::Lseek::ID => r#fn::lseek::Lseek::handle(frame),
        #[cfg(target_arch = "x86_64")]
        r#fn::openat::Open::ID => r#fn::openat::Open::handle(frame),
        r#fn::openat::OpenAt::ID => r#fn::openat::OpenAt::handle(frame),
        r#fn::clock_gettime::ClockGetTime::ID => {
            r#fn::clock_gettime::ClockGetTime::handle(frame)
        }
        r#fn::chdir::Chdir::ID => r#fn::chdir::Chdir::handle(frame),
        r#fn::brk::Brk::ID => r#fn::brk::Brk::handle(frame),
        r#fn::mmap::Mmap::ID => r#fn::mmap::Mmap::handle(frame),
        r#fn::munmap::Munmap::ID => r#fn::munmap::Munmap::handle(frame),
        r#fn::mprotect::Mprotect::ID => r#fn::mprotect::Mprotect::handle(frame),
        r#fn::nanosleep::Nanosleep::ID => r#fn::nanosleep::Nanosleep::handle(frame),
        r#fn::uname::Uname::ID => r#fn::uname::Uname::handle(frame),
        _ => r#fn::unsupported::handle_syscall(frame.number),
    }
}

pub(crate) fn dispatch(frame: &mut SyscallFrame) {
    ThreadManager::begin_syscall_accounting();
    frame.errno = 0;
    let result = dispatch_call(*frame);
    frame.result = match result {
        Ok(value) => value,
        Err(error) => error::to_syscall_result(error),
    };
    ThreadManager::end_syscall_accounting();
}

#[cfg(target_arch = "x86_64")]
pub(crate) fn dispatch_with_user_registers(
    frame: &mut SyscallFrame,
    registers: crate::kernel::arch::UserRegisterState,
) {
    ThreadManager::begin_syscall_accounting();
    frame.errno = 0;
    frame.result = match r#fn::fork::dispatch(*frame, registers) {
        Ok(value) => value,
        Err(error) => error::to_syscall_result(error),
    };
    ThreadManager::end_syscall_accounting();
}

#[cfg(test)]
#[path = "../../../tests/kernel/syscall/linux/dispatch.rs"]
mod tests;