/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::kernel::error::Error;
use crate::kernel::scheduler::sync::{IrqSpinGuard, IrqSpinLock};
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::tty::{Pty, TerminalAttributes, WindowSize};

pub(crate) const PTY_FD_BASE: i32 = 384;
const PTY_DESCRIPTOR_COUNT: usize = 32;
const PTY_PAIR_COUNT: usize = PTY_DESCRIPTOR_COUNT / 2;
const O_ACCMODE: i32 = 0o3;
const O_RDONLY: i32 = 0;
const O_WRONLY: i32 = 1;
const O_RDWR: i32 = 2;
const O_NONBLOCK: i32 = 0o4000;
const O_NOCTTY: i32 = 0o400;
const O_CLOEXEC: i32 = 0o2000000;

#[derive(Clone, Copy, Eq, PartialEq)]
enum PtySide {
    Master,
    Slave,
}

struct PtyOpenDescription {
    pty: Arc<Pty>,
    side: PtySide,
    descriptor_references: usize,
    nonblocking: bool,
    readable: bool,
    writable: bool,
}

struct PtyDescriptor {
    pty_index: usize,
    description: Arc<IrqSpinLock<PtyOpenDescription>>,
    close_on_exec: bool,
}

static PTYS: IrqSpinLock<[Option<Arc<Pty>>; PTY_PAIR_COUNT]> =
    IrqSpinLock::new([const { None }; PTY_PAIR_COUNT]);
static DESCRIPTORS: IrqSpinLock<[Option<PtyDescriptor>; PTY_DESCRIPTOR_COUNT]> =
    IrqSpinLock::new([const { None }; PTY_DESCRIPTOR_COUNT]);

fn descriptor_index(file_descriptor: i32) -> Result<usize, Error> {
    let file_descriptor = crate::kernel::runtime::runtime_libc::fd_alias::resolve(
        file_descriptor,
        crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind::Pty,
    )?;
    let index = file_descriptor
        .checked_sub(PTY_FD_BASE)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(Error::Badf)?;
    (index < PTY_DESCRIPTOR_COUNT)
        .then_some(index)
        .ok_or(Error::Badf)
}

pub(crate) fn is_fd(file_descriptor: i32) -> bool {
    if let Some(alias) = crate::kernel::runtime::runtime_libc::fd_alias::get(file_descriptor) {
        return alias.kind == crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind::Pty;
    }
    descriptor_index(file_descriptor).is_ok()
}

pub(crate) fn has_fd(file_descriptor: i32) -> bool {
    if let Some(alias) = crate::kernel::runtime::runtime_libc::fd_alias::get(file_descriptor) {
        return alias.kind == crate::kernel::runtime::runtime_libc::fd_alias::DescriptorKind::Pty
            && descriptor_index(alias.backing)
                .ok()
                .is_some_and(|index| DESCRIPTORS.lock()[index].is_some());
    }
    descriptor_index(file_descriptor)
        .ok()
        .is_some_and(|index| DESCRIPTORS.lock()[index].is_some())
}

fn available_descriptor_index(
    descriptors: &IrqSpinGuard<'_, [Option<PtyDescriptor>; PTY_DESCRIPTOR_COUNT]>,
    minimum: usize,
) -> Option<usize> {
    descriptors
        .iter()
        .enumerate()
        .skip(minimum)
        .find_map(|(index, descriptor)| {
            let file_descriptor = PTY_FD_BASE + index as i32;
            (descriptor.is_none()
                && !crate::kernel::runtime::runtime_libc::fd_alias::contains_target(
                    file_descriptor,
                ))
            .then_some(index)
        })
}

fn descriptor(file_descriptor: i32) -> Result<PtyDescriptor, Error> {
    DESCRIPTORS
        .lock()
        .get(descriptor_index(file_descriptor)?)
        .and_then(Option::as_ref)
        .map(|descriptor| PtyDescriptor {
            pty_index: descriptor.pty_index,
            description: Arc::clone(&descriptor.description),
            close_on_exec: descriptor.close_on_exec,
        })
        .ok_or(Error::Badf)
}

pub(crate) fn open_master(flags: i32) -> Result<i32, Error> {
    if flags & !(O_NONBLOCK | O_NOCTTY | O_CLOEXEC | O_ACCMODE) != 0
        || flags & O_ACCMODE != O_RDWR
    {
        return Err(Error::Inval);
    }
    let pty = Pty::new()?;
    let mut pairs = PTYS.lock();
    let pty_index = pairs.iter().position(Option::is_none).ok_or(Error::Mfile)?;
    let mut descriptors = DESCRIPTORS.lock();
    let descriptor_index = available_descriptor_index(&descriptors, 0).ok_or(Error::Mfile)?;
    crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(
        PTY_FD_BASE + descriptor_index as i32,
    )?;
    pairs[pty_index] = Some(Arc::clone(&pty));
    descriptors[descriptor_index] = Some(PtyDescriptor {
        pty_index,
        description: Arc::new(IrqSpinLock::new(PtyOpenDescription {
            pty,
            side: PtySide::Master,
            descriptor_references: 1,
            nonblocking: flags & O_NONBLOCK != 0,
            readable: true,
            writable: true,
        })),
        close_on_exec: flags & O_CLOEXEC != 0,
    });
    Ok(PTY_FD_BASE + descriptor_index as i32)
}

pub(crate) fn open_slave_path(path: &[u8], flags: i32) -> Result<i32, Error> {
    let pty_index = parse_pty_path(path)?;
    let pty = PTYS
        .lock()
        .get(pty_index)
        .and_then(Option::as_ref)
        .map(Arc::clone)
        .ok_or(Error::NoEnt)?;
    let descriptor = open_slave(pty_index, flags)?;
    if let Ok(process) = ThreadManager::current_process() {
        process.acquire_pty_terminal(Arc::clone(&pty), flags & O_NOCTTY != 0);
    }
    Ok(descriptor)
}

pub(crate) fn open_slave_for(
    terminal: Arc<Pty>,
    flags: i32,
) -> Result<i32, Error> {
    let pty_index = PTYS
        .lock()
        .iter()
        .position(|entry| entry.as_ref().is_some_and(|pty| Arc::ptr_eq(pty, &terminal)))
        .ok_or(Error::NoEnt)?;
    open_slave(pty_index, flags)
}

fn open_slave(pty_index: usize, flags: i32) -> Result<i32, Error> {
    if flags & !(O_NONBLOCK | O_NOCTTY | O_CLOEXEC | O_ACCMODE) != 0
        || flags & O_ACCMODE == O_ACCMODE
    {
        return Err(Error::Inval);
    }
    let pty = PTYS
        .lock()
        .get(pty_index)
        .and_then(Option::as_ref)
        .map(Arc::clone)
        .ok_or(Error::NoEnt)?;
    let mut descriptors = DESCRIPTORS.lock();
    let descriptor_index = available_descriptor_index(&descriptors, 0).ok_or(Error::Mfile)?;
    crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(
        PTY_FD_BASE + descriptor_index as i32,
    )?;
    pty.open_slave()?;
    descriptors[descriptor_index] = Some(PtyDescriptor {
        pty_index,
        description: Arc::new(IrqSpinLock::new(PtyOpenDescription {
            pty,
            side: PtySide::Slave,
            descriptor_references: 1,
            nonblocking: flags & O_NONBLOCK != 0,
            readable: flags & O_ACCMODE != O_WRONLY,
            writable: flags & O_ACCMODE != O_RDONLY,
        })),
        close_on_exec: flags & O_CLOEXEC != 0,
    });
    Ok(PTY_FD_BASE + descriptor_index as i32)
}

fn parse_pty_path(path: &[u8]) -> Result<usize, Error> {
    let Some(suffix) = path.strip_prefix(b"/dev/pts/") else {
        return Err(Error::NoEnt);
    };
    if suffix.is_empty() || suffix.iter().any(|byte| !byte.is_ascii_digit()) {
        return Err(Error::NoEnt);
    }
    let mut index = 0usize;
    for digit in suffix {
        index = index
            .checked_mul(10)
            .and_then(|value| value.checked_add(usize::from(*digit - b'0')))
            .ok_or(Error::NoEnt)?;
    }
    (index < PTY_PAIR_COUNT).then_some(index).ok_or(Error::NoEnt)
}

pub(crate) fn pty_index(file_descriptor: i32) -> Result<usize, Error> {
    let descriptor = descriptor(file_descriptor)?;
    if descriptor.description.lock().side != PtySide::Master {
        return Err(Error::Badf);
    }
    Ok(descriptor.pty_index)
}

pub(crate) fn device_index(file_descriptor: i32) -> Result<usize, Error> {
    Ok(descriptor(file_descriptor)?.pty_index)
}

pub(crate) fn unlock_slave(file_descriptor: i32) -> Result<(), Error> {
    let descriptor = descriptor(file_descriptor)?;
    let description = descriptor.description.lock();
    if description.side != PtySide::Master {
        return Err(Error::Badf);
    }
    description.pty.unlock_slave()
}

pub(crate) fn read(
    file_descriptor: i32,
    output: &mut [u8],
) -> Result<usize, Error> {
    let descriptor = descriptor(file_descriptor)?;
    let description = descriptor.description.lock();
    if !description.readable {
        return Err(Error::Badf);
    }
    match description.side {
        PtySide::Master => description.pty.master_read(output),
        PtySide::Slave => description.pty.slave_read(output),
    }
}

pub(crate) fn write(
    file_descriptor: i32,
    input: &[u8],
) -> Result<usize, Error> {
    let descriptor = descriptor(file_descriptor)?;
    let description = descriptor.description.lock();
    if !description.writable {
        return Err(Error::Badf);
    }
    match description.side {
        PtySide::Master => description.pty.master_write(input),
        PtySide::Slave => description.pty.slave_write(input),
    }
}

pub(crate) fn nonblocking(file_descriptor: i32) -> Result<bool, Error> {
    Ok(descriptor(file_descriptor)?.description.lock().nonblocking)
}

pub(crate) fn set_nonblocking(file_descriptor: i32, nonblocking: bool) -> Result<(), Error> {
    let descriptor = descriptor(file_descriptor)?;
    descriptor.description.lock().nonblocking = nonblocking;
    Ok(())
}

pub(crate) fn readiness(file_descriptor: i32, events: i16) -> Result<i16, Error> {
    let descriptor = descriptor(file_descriptor)?;
    let description = descriptor.description.lock();
    match description.side {
        PtySide::Master => description.pty.master_readiness(events),
        PtySide::Slave => description.pty.slave_readiness(events),
    }
}

pub(crate) fn attributes(file_descriptor: i32) -> Result<TerminalAttributes, Error> {
    descriptor(file_descriptor)?.description.lock().pty.attributes()
}

pub(crate) fn set_attributes(
    file_descriptor: i32,
    attributes: TerminalAttributes,
) -> Result<(), Error> {
    descriptor(file_descriptor)?
        .description
        .lock()
        .pty
        .set_attributes(attributes)
}

pub(crate) fn window_size(file_descriptor: i32) -> Result<WindowSize, Error> {
    descriptor(file_descriptor)?.description.lock().pty.window_size()
}

pub(crate) fn set_window_size(file_descriptor: i32, size: WindowSize) -> Result<(), Error> {
    descriptor(file_descriptor)?
        .description
        .lock()
        .pty
        .set_window_size(size)
}

pub(crate) fn flush_input(file_descriptor: i32) -> Result<(), Error> {
    descriptor(file_descriptor)?
        .description
        .lock()
        .pty
        .flush_input();
    Ok(())
}

pub(crate) fn flush_output(file_descriptor: i32) -> Result<(), Error> {
    descriptor(file_descriptor)?
        .description
        .lock()
        .pty
        .flush_output();
    Ok(())
}

pub(crate) fn flow(file_descriptor: i32, action: i32) -> Result<(), Error> {
    descriptor(file_descriptor)?
        .description
        .lock()
        .pty
        .flow(action)
}

pub(crate) fn descriptor_flags(file_descriptor: i32) -> Result<i32, Error> {
    Ok(i32::from(descriptor(file_descriptor)?.close_on_exec))
}

pub(crate) fn set_descriptor_flags(file_descriptor: i32, flags: i32) -> Result<(), Error> {
    if flags & !1 != 0 {
        return Err(Error::Inval);
    }
    let index = descriptor_index(file_descriptor)?;
    DESCRIPTORS.lock()[index]
        .as_mut()
        .ok_or(Error::Badf)?
        .close_on_exec = flags & 1 != 0;
    Ok(())
}

pub(crate) fn status_flags(file_descriptor: i32) -> Result<i32, Error> {
    let descriptor = descriptor(file_descriptor)?;
    let description = descriptor.description.lock();
    let access = match (description.readable, description.writable) {
        (true, true) => O_RDWR,
        (true, false) => O_RDONLY,
        (false, true) => O_WRONLY,
        (false, false) => return Err(Error::Badf),
    };
    Ok(access | if description.nonblocking { O_NONBLOCK } else { 0 })
}

pub(crate) fn set_status_flags(file_descriptor: i32, flags: i32) -> Result<(), Error> {
    if flags & !(O_NONBLOCK | O_ACCMODE) != 0 {
        return Err(Error::Inval);
    }
    set_nonblocking(file_descriptor, flags & O_NONBLOCK != 0)
}

pub(crate) fn duplicate(
    file_descriptor: i32,
    minimum: i32,
    close_on_exec: bool,
) -> Result<i32, Error> {
    duplicate_internal(file_descriptor, minimum, close_on_exec, true)
}

pub(crate) fn duplicate_backing(file_descriptor: i32) -> Result<i32, Error> {
    duplicate_internal(file_descriptor, PTY_FD_BASE, false, false)
}

fn duplicate_internal(
    file_descriptor: i32,
    minimum: i32,
    close_on_exec: bool,
    enforce_limit: bool,
) -> Result<i32, Error> {
    if minimum < 0 {
        return Err(Error::Inval);
    }
    let source_index = descriptor_index(file_descriptor)?;
    let mut descriptors = DESCRIPTORS.lock();
    let source = descriptors[source_index].as_ref().ok_or(Error::Badf)?;
    let minimum_index = usize::try_from(minimum.saturating_sub(PTY_FD_BASE))
        .unwrap_or(0)
        .min(PTY_DESCRIPTOR_COUNT);
    let target_index = available_descriptor_index(&descriptors, minimum_index).ok_or(Error::Mfile)?;
    if enforce_limit {
        crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(
            PTY_FD_BASE + target_index as i32,
        )?;
    }
    let description = Arc::clone(&source.description);
    let mut open_description = description.lock();
    open_description.descriptor_references = open_description
        .descriptor_references
        .checked_add(1)
        .ok_or(Error::Mfile)?;
    drop(open_description);
    descriptors[target_index] = Some(PtyDescriptor {
        pty_index: source.pty_index,
        description,
        close_on_exec,
    });
    Ok(PTY_FD_BASE + target_index as i32)
}

pub(crate) fn duplicate_to(
    file_descriptor: i32,
    new_descriptor: i32,
    close_on_exec: bool,
) -> Result<i32, Error> {
    let source_index = descriptor_index(file_descriptor)?;
    let target_index = descriptor_index(new_descriptor)?;
    crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(new_descriptor)?;
    if file_descriptor == new_descriptor {
        return if close_on_exec {
            Err(Error::Inval)
        } else {
            Ok(new_descriptor)
        };
    }
    let replaced = {
        let mut descriptors = DESCRIPTORS.lock();
        let source = descriptors[source_index].as_ref().ok_or(Error::Badf)?;
        let description = Arc::clone(&source.description);
        let mut open_description = description.lock();
        open_description.descriptor_references = open_description
            .descriptor_references
            .checked_add(1)
            .ok_or(Error::Mfile)?;
        drop(open_description);
        let replacement = PtyDescriptor {
            pty_index: source.pty_index,
            description,
            close_on_exec,
        };
        descriptors[target_index].replace(replacement)
    };
    if let Some(replaced) = replaced {
        close_entry(replaced);
    }
    Ok(new_descriptor)
}

pub(crate) fn close(file_descriptor: i32) -> Result<(), Error> {
    let index = descriptor_index(file_descriptor)?;
    let descriptor = DESCRIPTORS.lock()[index].take().ok_or(Error::Badf)?;
    close_entry(descriptor);
    Ok(())
}

pub(crate) fn close_on_exec_descriptors() -> Result<(), Error> {
    let descriptors = {
        let descriptors = DESCRIPTORS.lock();
        let mut close_descriptors = Vec::new();
        close_descriptors
            .try_reserve(descriptors.iter().flatten().filter(|fd| fd.close_on_exec).count())
            .map_err(|_| Error::NoMem)?;
        close_descriptors.extend(descriptors.iter().enumerate().filter_map(|(index, fd)| {
            fd.as_ref()
                .is_some_and(|fd| fd.close_on_exec)
                .then_some(PTY_FD_BASE + index as i32)
        }));
        close_descriptors
    };
    for file_descriptor in descriptors {
        let _ = close(file_descriptor);
    }
    Ok(())
}

fn close_entry(descriptor: PtyDescriptor) {
    let endpoint_close = {
        let mut description = descriptor.description.lock();
        description.descriptor_references = description.descriptor_references.saturating_sub(1);
        (description.descriptor_references == 0)
            .then(|| (Arc::clone(&description.pty), description.side))
    };
    if let Some((pty, side)) = endpoint_close {
        match side {
            PtySide::Master => pty.close_master(),
            PtySide::Slave => pty.close_slave(),
        }
    }
    let still_open = DESCRIPTORS
        .lock()
        .iter()
        .flatten()
        .any(|entry| entry.pty_index == descriptor.pty_index);
    if !still_open {
        PTYS.lock()[descriptor.pty_index] = None;
    }
}