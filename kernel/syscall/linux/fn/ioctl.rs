/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec;

use super::super::SyscallFrame;
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::vfs::FileHandleApi as _;

const TIOCGPGRP: u64 = crate::kernel::vfs::filesystem::tty::TIOCGPGRP;
const TIOCSPGRP: u64 = crate::kernel::vfs::filesystem::tty::TIOCSPGRP;
const TIOCGWINSZ: u64 = crate::kernel::vfs::filesystem::tty::TIOCGWINSZ;
const TIOCSWINSZ: u64 = crate::kernel::vfs::filesystem::tty::TIOCSWINSZ;
const TCGETS: u64 = crate::kernel::vfs::filesystem::tty::TCGETS;
const TCSETS: u64 = crate::kernel::vfs::filesystem::tty::TCSETS;
const TCSETSW: u64 = crate::kernel::vfs::filesystem::tty::TCSETSW;
const TCSETSF: u64 = crate::kernel::vfs::filesystem::tty::TCSETSF;
const TCSBRK: u64 = crate::kernel::vfs::filesystem::tty::TCSBRK;
const TCXONC: u64 = crate::kernel::vfs::filesystem::tty::TCXONC;
const TCFLSH: u64 = crate::kernel::vfs::filesystem::tty::TCFLSH;

pub(crate) struct Ioctl;

impl Ioctl {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Ioctl;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let descriptor = u32::try_from(frame.arguments[0]).map_err(|_| Error::Badf)?;
        let request = frame.arguments[1];
        let length = argument_length(request).ok_or(Error::NoSys)?;
        let address = usize::try_from(frame.arguments[2]).map_err(|_| Error::Fault)?;
        let process = ThreadManager::current_process()?;
        let mut argument = vec![0; length];
        let output_only = matches!(request, TIOCGPGRP | TIOCGWINSZ | TCGETS);
        if !output_only {
            process
                .address_space()
                .read(address, &mut argument)
                .map_err(|_| Error::Fault)?;
        }

        let result = if descriptor < 3 {
            crate::kernel::tty::ioctl_console(request, &mut argument)?
        } else {
            let object = process.file_descriptor(descriptor)?;
            let mut file = object.take_file()?;
            let result = file.ioctl_blocking(request, &mut argument, &process.credentials()?);
            object.restore_file(file)?;
            result?
        };

        if output_only || !matches!(request, TCSBRK | TCXONC | TCFLSH) {
            process
                .address_space()
                .write(address, &argument)
                .map_err(|_| Error::Fault)?;
        }
        Ok(result)
    }
}

fn argument_length(request: u64) -> Option<usize> {
    Some(match request {
        TIOCGPGRP | TIOCSPGRP | TCSBRK | TCXONC | TCFLSH => 4,
        TIOCGWINSZ | TIOCSWINSZ => 8,
        TCGETS | TCSETS | TCSETSW | TCSETSF => 36,
        _ => return None,
    })
}