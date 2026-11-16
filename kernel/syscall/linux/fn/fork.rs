/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::arch::UserRegisterState;
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::signal::SIGCHLD;
use crate::kernel::vfs::{OpenMode, VfsApi};

pub(crate) struct ForkProcess;
pub(crate) struct CloneProcess;
pub(crate) struct VforkProcess;

impl ForkProcess {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Fork;
}

impl CloneProcess {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Clone;
}

impl VforkProcess {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Vfork;
}

pub(crate) fn dispatch(
    frame: SyscallFrame,
    registers: UserRegisterState,
) -> Result<i64, Error> {
    match frame.number {
        ForkProcess::ID => fork_process(registers),
        VforkProcess::ID => fork_process(registers),
        CloneProcess::ID => {
            validate_clone_arguments(frame.arguments)?;
            fork_process(registers)
        }
        _ => Err(Error::NoSys),
    }
}

fn validate_clone_arguments(arguments: [u64; 6]) -> Result<(), Error> {
    if arguments[0] != u64::from(SIGCHLD)
        || arguments[1..5].iter().any(|argument| *argument != 0)
    {
        return Err(Error::Inval);
    }
    Ok(())
}

fn probe_vfs_root_lookup(process: &Process, stage: &str) {
    crate::kernel::debug::debug_fmt(format_args!(
        "kz: fork VFS lookup probe start stage={} pid={}\n",
        stage,
        process.pid()
    ));
    let success = match (crate::kernel::vfs::mounted(), process.credentials()) {
        (Some(vfs), Ok(credentials)) => {
            let result = vfs.open_blocking("/fs", OpenMode::ReadOnly, credentials);
            let success = result.is_ok();
            drop(result);
            success
        }
        _ => false,
    };
    crate::kernel::debug::debug_fmt(format_args!(
        "kz: fork VFS lookup probe complete stage={} pid={} success={}\n",
        stage,
        process.pid(),
        success
    ));
}

fn fork_process(mut registers: UserRegisterState) -> Result<i64, Error> {
    let parent = ThreadManager::current_process()?;
    let (_, signal_mask) = ThreadManager::current_signal_context()?;
    crate::kernel::debug::debug_fmt(format_args!(
        "kz: fork source address space pid={} root={:#x}\n",
        parent.pid(), parent.address_space().root()
    ));
    let address_space = parent.address_space().fork_clone()?;
    crate::kernel::debug::debug_fmt(format_args!(
        "kz: fork clone complete parent={} root={:#x} child={:#x}\n",
        parent.pid(),
        parent.address_space().root(),
        address_space.root()
    ));
    probe_vfs_root_lookup(&parent, "after-clone");
    let child = Process::new_fork_child(&parent, address_space)?;
    let child_pid = child.pid();
    probe_vfs_root_lookup(&parent, "after-process");
    registers.rax = 0;
    if let Err(error) = ThreadManager::create_fork_child(child, registers, signal_mask) {
        parent.discard_unstarted_child(child_pid);
        return Err(error);
    }
    probe_vfs_root_lookup(&parent, "after-thread");
    crate::kernel::debug::debug_fmt(format_args!(
        "kz: fork enqueued parent={} child={}\n",
        parent.pid(), child_pid
    ));
    Ok(i64::from(child_pid))
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/fork.rs"]
mod tests;