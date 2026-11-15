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

fn fork_process(mut registers: UserRegisterState) -> Result<i64, Error> {
    crate::kernel::debug::debug("kz: fork syscall entered\n");
    let parent = ThreadManager::current_process()?;
    let (_, signal_mask) = ThreadManager::current_signal_context()?;
    let free_frames_before = crate::kernel::memory::physical::PhysicalAllocator::free_frames();
    crate::kernel::debug::debug_fmt(format_args!(
        "kz: fork clone starting, free frames={}\n",
        free_frames_before
    ));
    let address_space = match parent.address_space().fork_clone() {
        Ok(address_space) => address_space,
        Err(error) => {
            crate::kernel::debug::debug_fmt(format_args!(
                "kz: fork address-space clone failed {:?}, free frames {} -> {}\n",
                error,
                free_frames_before,
                crate::kernel::memory::physical::PhysicalAllocator::free_frames()
            ));
            return Err(error);
        }
    };
    crate::kernel::debug::debug("kz: fork address-space clone complete\n");
    let child = match Process::new_fork_child(&parent, address_space) {
        Ok(child) => child,
        Err(error) => {
            crate::kernel::debug::debug_fmt(format_args!(
                "kz: fork process creation failed {:?}, free frames {}\n",
                error,
                crate::kernel::memory::physical::PhysicalAllocator::free_frames()
            ));
            return Err(error);
        }
    };
    crate::kernel::debug::debug("kz: fork child process registered\n");
    let child_pid = child.pid();
    registers.rax = 0;
    if let Err(error) = ThreadManager::create_fork_child(child, registers, signal_mask) {
        crate::kernel::debug::debug_fmt(format_args!(
            "kz: fork child-thread creation failed {:?}, free frames {}\n",
            error,
            crate::kernel::memory::physical::PhysicalAllocator::free_frames()
        ));
        parent.discard_unstarted_child(child_pid);
        return Err(error);
    }
    crate::kernel::debug::debug("kz: fork child thread enqueued\n");
    Ok(i64::from(child_pid))
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/fork.rs"]
mod tests;