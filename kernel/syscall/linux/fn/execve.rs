/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::mem::size_of;

use super::super::SyscallFrame;
use super::path::{copy_user_path, resolve_at_current_directory};
use crate::kernel::dl::AddressSpace as _;
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::{boot, vm::UserAddressSpace, ThreadManager};

const MAX_VECTOR_ENTRIES: usize = 4096;
const MAX_ARGUMENT_BYTES: usize = 512 * 1024;
const USER_COPY_CHUNK: usize = 64;

pub(crate) struct Execve;

impl Execve {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Execve;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let process = ThreadManager::current_process()?;
        crate::kernel::debug::debug_fmt(format_args!(
            "kz: execve entered pid={}\n",
            process.pid()
        ));
        let address_space = process.address_space();
        let path_address = usize::try_from(frame.arguments[0]).map_err(|_| Error::Fault)?;
        let path = resolve_at_current_directory(
            copy_user_path(&address_space, path_address)?,
            &process.current_directory()?,
        )?;
        crate::kernel::debug::debug_fmt(format_args!(
            "kz: execve resolved pid={} path={}\n",
            process.pid(), path
        ));
        let mut argument_bytes = 0;
        let arguments = copy_user_strings(
            &address_space,
            usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?,
            &mut argument_bytes,
        )?;
        let environment = copy_user_strings(
            &address_space,
            usize::try_from(frame.arguments[2]).map_err(|_| Error::Fault)?,
            &mut argument_bytes,
        )?;
        let argument_slices = string_slices(&arguments)?;
        let environment_slices = string_slices(&environment)?;

        crate::kernel::debug::debug_fmt(format_args!(
            "kz: execve loading pid={} path={}\n",
            process.pid(), path
        ));
        boot::exec_image(
            &path,
            argument_slices.as_slice(),
            environment_slices.as_slice(),
        )?;
        Ok(0)
    }
}

fn copy_user_strings(
    address_space: &UserAddressSpace,
    vector_address: usize,
    total_bytes: &mut usize,
) -> Result<Vec<Vec<u8>>, Error> {
    let mut strings = Vec::new();
    strings.try_reserve(8).map_err(|_| Error::NoMem)?;
    if vector_address == 0 {
        return Ok(strings);
    }

    for index in 0..MAX_VECTOR_ENTRIES {
        let offset = index
            .checked_mul(size_of::<usize>())
            .ok_or(Error::Fault)?;
        let address = vector_address.checked_add(offset).ok_or(Error::Fault)?;
        let mut pointer_bytes = [0; size_of::<usize>()];
        address_space
            .read(address, &mut pointer_bytes)
            .map_err(|_| Error::Fault)?;
        let string_address = usize::from_ne_bytes(pointer_bytes);
        if string_address == 0 {
            return Ok(strings);
        }
        strings.try_reserve(1).map_err(|_| Error::NoMem)?;
        strings.push(copy_user_string(address_space, string_address, total_bytes)?);
    }
    Err(Error::TooBig)
}

fn copy_user_string(
    address_space: &UserAddressSpace,
    address: usize,
    total_bytes: &mut usize,
) -> Result<Vec<u8>, Error> {
    let remaining = MAX_ARGUMENT_BYTES
        .checked_sub(*total_bytes)
        .filter(|remaining| *remaining != 0)
        .ok_or(Error::TooBig)?;
    let mut string = Vec::new();
    string.try_reserve(USER_COPY_CHUNK).map_err(|_| Error::NoMem)?;
    while string.len() < remaining {
        let current = address.checked_add(string.len()).ok_or(Error::Fault)?;
        let page_remaining = crate::kernel::memory::r#virtual::PAGE_SIZE
            - current % crate::kernel::memory::r#virtual::PAGE_SIZE;
        let chunk_length = USER_COPY_CHUNK
            .min(page_remaining)
            .min(remaining - string.len());
        let mut chunk = [0; USER_COPY_CHUNK];
        address_space
            .read(current, &mut chunk[..chunk_length])
            .map_err(|_| Error::Fault)?;
        let length = chunk[..chunk_length]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(chunk_length);
        string.try_reserve(length).map_err(|_| Error::NoMem)?;
        string.extend_from_slice(&chunk[..length]);
        if length < chunk_length {
            *total_bytes = total_bytes
                .checked_add(string.len().checked_add(1).ok_or(Error::TooBig)?)
                .ok_or(Error::TooBig)?;
            return Ok(string);
        }
    }
    Err(Error::TooBig)
}

fn string_slices(strings: &[Vec<u8>]) -> Result<Vec<&[u8]>, Error> {
    let mut slices = Vec::new();
    slices
        .try_reserve_exact(strings.len())
        .map_err(|_| Error::NoMem)?;
    slices.extend(strings.iter().map(Vec::as_slice));
    Ok(slices)
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/execve.rs"]
mod tests;