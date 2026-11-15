/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::string::String;
use alloc::vec::Vec;

use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::scheduler::thread::vm::UserAddressSpace;
use crate::kernel::vfs::FileHandleApi as _;

const MAX_PATH_LENGTH: usize = 4096;
const PATH_COPY_CHUNK: usize = 64;

pub(crate) fn copy_user_path(
    address_space: &UserAddressSpace,
    address: usize,
) -> Result<String, Error> {
    if address == 0 {
        return Err(Error::Fault);
    }
    let mut path = Vec::new();
    path.try_reserve(128).map_err(|_| Error::NoMem)?;
    while path.len() < MAX_PATH_LENGTH {
        let current = address.checked_add(path.len()).ok_or(Error::Fault)?;
        let page_remaining = crate::kernel::memory::r#virtual::PAGE_SIZE
            - current % crate::kernel::memory::r#virtual::PAGE_SIZE;
        let chunk_length = PATH_COPY_CHUNK
            .min(page_remaining)
            .min(MAX_PATH_LENGTH - path.len());
        let mut chunk = [0; PATH_COPY_CHUNK];
        address_space
            .read(current, &mut chunk[..chunk_length])
            .map_err(|_| Error::Fault)?;
        let length = chunk[..chunk_length]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(chunk_length);
        path.try_reserve(length).map_err(|_| Error::NoMem)?;
        path.extend_from_slice(&chunk[..length]);
        if length < chunk_length {
            return String::from_utf8(path).map_err(|_| Error::Inval);
        }
    }
    Err(Error::NameTooLong)
}

pub(crate) fn resolve_at_current_directory(
    path: String,
    current_directory: &[u8],
) -> Result<String, Error> {
    if path.is_empty() {
        return Err(Error::NoEnt);
    }
    if path.starts_with('/') {
        return (path.len() <= MAX_PATH_LENGTH)
            .then_some(path)
            .ok_or(Error::NameTooLong);
    }
    let current_directory = core::str::from_utf8(current_directory).map_err(|_| Error::Inval)?;
    if !current_directory.starts_with('/') {
        return Err(Error::Inval);
    }
    let separator_length = usize::from(current_directory != "/");
    let total_length = current_directory
        .len()
        .checked_add(separator_length)
        .and_then(|length| length.checked_add(path.len()))
        .ok_or(Error::NameTooLong)?;
    if total_length > MAX_PATH_LENGTH {
        return Err(Error::NameTooLong);
    }
    let mut resolved = String::new();
    resolved
        .try_reserve_exact(total_length)
        .map_err(|_| Error::NoMem)?;
    resolved.push_str(current_directory);
    if separator_length != 0 {
        resolved.push('/');
    }
    resolved.push_str(&path);
    Ok(resolved)
}

pub(crate) fn resolve_at_directory(
    path: String,
    directory_file_descriptor: u64,
    process: &Process,
) -> Result<String, Error> {
    if path.is_empty() {
        return Err(Error::NoEnt);
    }
    if path.starts_with('/') {
        return resolve_at_current_directory(path, b"/");
    }
    if directory_file_descriptor as i32 == -100 {
        return resolve_at_current_directory(path, &process.current_directory()?);
    }
    let descriptor = u32::try_from(directory_file_descriptor as i32).map_err(|_| Error::Badf)?;
    let object = process.file_descriptor(descriptor)?;
    let file = object.take_file()?;
    let is_directory = file.is_directory();
    let resolved = if is_directory {
        resolve_at_current_directory(path, file.path().as_bytes())
    } else {
        Err(Error::NotDir)
    };
    object.restore_file(file)?;
    resolved
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/path.rs"]
mod tests;