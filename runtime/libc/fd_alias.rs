/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use crate::kernel::error::Error;
use crate::kernel::scheduler::sync::IrqSpinLock;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum DescriptorKind {
    File,
    Socket,
    Packet,
    Pty,
}

#[derive(Clone, Copy)]
pub(crate) struct DescriptorAlias {
    pub(crate) target: i32,
    pub(crate) backing: i32,
    pub(crate) kind: DescriptorKind,
    pub(crate) close_on_exec: bool,
}

#[derive(Clone, Copy)]
struct ProcessAlias {
    process_id: i32,
    alias: DescriptorAlias,
}

#[derive(Clone, Copy)]
struct ProcessReservation {
    process_id: i32,
    target: i32,
}

static ALIASES: IrqSpinLock<Vec<ProcessAlias>> = IrqSpinLock::new(Vec::new());
static RESERVED_TARGETS: IrqSpinLock<Vec<ProcessReservation>> = IrqSpinLock::new(Vec::new());

fn current_process_id() -> Option<i32> {
    crate::kernel::scheduler::thread::ThreadManager::current_process()
        .ok()
        .map(|process| process.pid())
}

pub(crate) fn get(target: i32) -> Option<DescriptorAlias> {
    get_for_process(current_process_id()?, target)
}

pub(crate) fn get_for_process(process_id: i32, target: i32) -> Option<DescriptorAlias> {
    ALIASES
        .lock()
        .iter()
        .find(|entry| entry.process_id == process_id && entry.alias.target == target)
        .map(|entry| entry.alias)
}

pub(crate) fn is_backing_for_process(process_id: i32, backing: i32) -> bool {
    ALIASES.lock().iter().any(|entry| {
        entry.process_id == process_id
            && entry.alias.backing == backing
            && entry.alias.target != backing
    })
}

pub(crate) fn replace_backing_for_process(
    process_id: i32,
    target: i32,
    old_backing: i32,
    new_backing: i32,
) -> Result<(), Error> {
    let mut aliases = ALIASES.lock();
    let entry = aliases
        .iter_mut()
        .find(|entry| entry.process_id == process_id && entry.alias.target == target)
        .ok_or(Error::Badf)?;
    if entry.alias.backing != old_backing {
        return Err(Error::Badf);
    }
    entry.alias.backing = new_backing;
    Ok(())
}

pub(crate) fn contains_target(target: i32) -> bool {
    current_process_id().is_some_and(|process_id| contains_target_for_process(process_id, target))
}

pub(crate) fn contains_target_for_process(process_id: i32, target: i32) -> bool {
    ALIASES
        .lock()
        .iter()
        .any(|entry| entry.process_id == process_id && entry.alias.target == target)
        || RESERVED_TARGETS
            .lock()
            .iter()
            .any(|entry| entry.process_id == process_id && entry.target == target)
}

pub(crate) fn reserve_targets_for_process(
    process_id: i32,
    targets: &[i32],
) -> Result<(), Error> {
    let mut reservations = RESERVED_TARGETS.lock();
    let additional = targets
        .iter()
        .filter(|target| {
            !reservations
                .iter()
                .any(|entry| entry.process_id == process_id && entry.target == **target)
        })
        .count();
    reservations.try_reserve(additional).map_err(|_| Error::NoMem)?;
    for target in targets {
        if !reservations
            .iter()
            .any(|entry| entry.process_id == process_id && entry.target == *target)
        {
            reservations.push(ProcessReservation {
                process_id,
                target: *target,
            });
        }
    }
    Ok(())
}

pub(crate) fn release_reserved_targets_for_process(process_id: i32) {
    RESERVED_TARGETS
        .lock()
        .retain(|entry| entry.process_id != process_id);
}

pub(crate) fn aliases_for_process(process_id: i32) -> Result<Vec<DescriptorAlias>, Error> {
    let aliases = ALIASES.lock();
    let mut process_aliases = Vec::new();
    process_aliases
        .try_reserve_exact(
            aliases
                .iter()
                .filter(|entry| entry.process_id == process_id)
                .count(),
        )
        .map_err(|_| Error::NoMem)?;
    process_aliases.extend(
        aliases
            .iter()
            .filter(|entry| entry.process_id == process_id)
            .map(|entry| entry.alias),
    );
    Ok(process_aliases)
}

pub(crate) fn close_on_exec_targets() -> Result<Vec<i32>, Error> {
    let process_id = current_process_id().ok_or(Error::Srch)?;
    close_on_exec_targets_for_process(process_id)
}

pub(crate) fn close_on_exec_targets_for_process(process_id: i32) -> Result<Vec<i32>, Error> {
    let aliases = ALIASES.lock();
    let mut targets = Vec::new();
    targets
        .try_reserve(
            aliases
                .iter()
                .filter(|entry| {
                    entry.process_id == process_id && entry.alias.close_on_exec
                })
                .count(),
        )
        .map_err(|_| Error::NoMem)?;
    targets.extend(
        aliases
            .iter()
            .filter(|entry| entry.process_id == process_id && entry.alias.close_on_exec)
            .map(|entry| entry.alias.target),
    );
    Ok(targets)
}

pub(crate) fn resolve_any(target: i32) -> i32 {
    get(target).map_or(target, |alias| alias.backing)
}

pub(crate) fn resolve_for_process(
    process_id: i32,
    target: i32,
    expected: DescriptorKind,
) -> Result<i32, Error> {
    match get_for_process(process_id, target) {
        Some(alias) if alias.kind == expected => Ok(alias.backing),
        Some(_) => Err(Error::Badf),
        None if is_backing_for_process(process_id, target) => Err(Error::Badf),
        None => Ok(target),
    }
}

pub(crate) fn resolve_any_for_process(process_id: i32, target: i32) -> i32 {
    get_for_process(process_id, target).map_or_else(
        || if is_backing_for_process(process_id, target) { -1 } else { target },
        |alias| alias.backing,
    )
}

pub(crate) fn reserve_one() -> Result<(), Error> {
    let mut aliases = ALIASES.lock();
    aliases.try_reserve(1).map_err(|_| Error::NoMem)
}

pub(crate) fn resolve(target: i32, expected: DescriptorKind) -> Result<i32, Error> {
    match get(target) {
        Some(alias) if alias.kind == expected => Ok(alias.backing),
        Some(_) => Err(Error::Badf),
        None if current_process_id()
            .is_some_and(|process_id| is_backing_for_process(process_id, target)) =>
        {
            Err(Error::Badf)
        }
        None => Ok(target),
    }
}

pub(crate) fn install(alias: DescriptorAlias) -> Result<(), Error> {
    let process_id = current_process_id().ok_or(Error::Srch)?;
    install_for_process(process_id, alias)
}

pub(crate) fn install_for_process(
    process_id: i32,
    alias: DescriptorAlias,
) -> Result<(), Error> {
    let mut aliases = ALIASES.lock();
    if aliases.iter().any(|existing| {
        existing.process_id == process_id && existing.alias.target == alias.target
    }) {
        return Err(Error::Exist);
    }
    aliases.try_reserve(1).map_err(|_| Error::NoMem)?;
    aliases.push(ProcessAlias { process_id, alias });
    Ok(())
}

pub(crate) fn remove(target: i32) -> Option<DescriptorAlias> {
    remove_for_process(current_process_id()?, target)
}

pub(crate) fn remove_for_process(process_id: i32, target: i32) -> Option<DescriptorAlias> {
    let mut aliases = ALIASES.lock();
    let index = aliases
        .iter()
        .position(|entry| entry.process_id == process_id && entry.alias.target == target)?;
    Some(aliases.remove(index).alias)
}

pub(crate) fn take_one_for_process(process_id: i32) -> Option<DescriptorAlias> {
    let mut aliases = ALIASES.lock();
    let index = aliases
        .iter()
        .position(|entry| entry.process_id == process_id)?;
    Some(aliases.remove(index).alias)
}

pub(crate) fn remove_all_for_process(process_id: i32) -> Result<Vec<DescriptorAlias>, Error> {
    let mut aliases = ALIASES.lock();
    let mut process_aliases = Vec::new();
    process_aliases
        .try_reserve_exact(
            aliases
                .iter()
                .filter(|entry| entry.process_id == process_id)
                .count(),
        )
        .map_err(|_| Error::NoMem)?;
    process_aliases.extend(
        aliases
            .iter()
            .filter(|entry| entry.process_id == process_id)
            .map(|entry| entry.alias),
    );
    aliases.retain(|entry| entry.process_id != process_id);
    RESERVED_TARGETS
        .lock()
        .retain(|entry| entry.process_id != process_id);
    Ok(process_aliases)
}

pub(crate) fn set_close_on_exec(target: i32, enabled: bool) -> Result<(), Error> {
    let process_id = current_process_id().ok_or(Error::Srch)?;
    set_close_on_exec_for_process(process_id, target, enabled)
}

pub(crate) fn set_close_on_exec_for_process(
    process_id: i32,
    target: i32,
    enabled: bool,
) -> Result<(), Error> {
    let mut aliases = ALIASES.lock();
    let entry = aliases
        .iter_mut()
        .find(|entry| entry.process_id == process_id && entry.alias.target == target)
        .ok_or(Error::Badf)?;
    entry.alias.close_on_exec = enabled;
    Ok(())
}
