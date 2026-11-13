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

static ALIASES: IrqSpinLock<Vec<DescriptorAlias>> = IrqSpinLock::new(Vec::new());

pub(crate) fn get(target: i32) -> Option<DescriptorAlias> {
    ALIASES
        .lock()
        .iter()
        .find(|alias| alias.target == target)
        .copied()
}

pub(crate) fn contains_target(target: i32) -> bool {
    ALIASES
        .lock()
        .iter()
        .any(|alias| alias.target == target)
}

pub(crate) fn resolve_any(target: i32) -> i32 {
    get(target).map_or(target, |alias| alias.backing)
}

pub(crate) fn reserve_one() -> Result<(), Error> {
    let mut aliases = ALIASES.lock();
    aliases.try_reserve(1).map_err(|_| Error::NoMem)
}

pub(crate) fn resolve(target: i32, expected: DescriptorKind) -> Result<i32, Error> {
    match get(target) {
        Some(alias) if alias.kind == expected => Ok(alias.backing),
        Some(_) => Err(Error::Badf),
        None => Ok(target),
    }
}

pub(crate) fn install(alias: DescriptorAlias) -> Result<(), Error> {
    let mut aliases = ALIASES.lock();
    if aliases.iter().any(|existing| existing.target == alias.target) {
        return Err(Error::Exist);
    }
    aliases.try_reserve(1).map_err(|_| Error::NoMem)?;
    aliases.push(alias);
    Ok(())
}

pub(crate) fn remove(target: i32) -> Option<DescriptorAlias> {
    let mut aliases = ALIASES.lock();
    let index = aliases.iter().position(|alias| alias.target == target)?;
    Some(aliases.remove(index))
}

pub(crate) fn set_close_on_exec(target: i32, enabled: bool) -> Result<(), Error> {
    let mut aliases = ALIASES.lock();
    let alias = aliases
        .iter_mut()
        .find(|alias| alias.target == target)
        .ok_or(Error::Badf)?;
    alias.close_on_exec = enabled;
    Ok(())
}
