/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::mem::size_of;

use crate::kernel::dl::{
    AddressSpace, ElfKind, LoadedObject, SliceFile, SliceFileApi as _,
};
use crate::kernel::error::Error;
use crate::kernel::scheduler::thread::vm::UserAddressSpace;
use crate::kernel::vfs::{FileHandleApi as _, OpenMode, VfsApi as _};

const PAGE_SIZE: usize = 4096;
const MAX_EXECUTABLE_SIZE: usize = 64 * 1024 * 1024;
const DEFAULT_STACK_SIZE: usize = 1024 * 1024;
const AT_PHDR: usize = 3;
const AT_PHENT: usize = 4;
const AT_PHNUM: usize = 5;
const AT_PAGESZ: usize = 6;
const AT_BASE: usize = 7;
const AT_FLAGS: usize = 8;
const AT_ENTRY: usize = 9;
const AT_UID: usize = 11;
const AT_EUID: usize = 12;
const AT_GID: usize = 13;
const AT_EGID: usize = 14;
const AT_PLATFORM: usize = 15;
const AT_HWCAP: usize = 16;
const AT_CLKTCK: usize = 17;
const AT_SECURE: usize = 23;
const AT_RANDOM: usize = 25;
const AT_HWCAP2: usize = 26;
const AT_EXECFN: usize = 31;
const AT_NULL: usize = 0;
const PT_LOAD: u32 = 1;
const PT_PHDR: u32 = 6;

pub(crate) fn launch_busybox() -> Result<(), Error> {
    let process = crate::kernel::scheduler::thread::ThreadManager::current_process()?;
    let vfs = crate::kernel::vfs::mounted().ok_or(Error::NoDev)?;
    let mut image_file = vfs.open_blocking(
        "/fs/busybox",
        OpenMode::ReadOnly,
        process.credentials()?,
    )?;
    let image_length = usize::try_from(image_file.size_blocking()?).map_err(|_| Error::Fbig)?;
    if image_length == 0 || image_length > MAX_EXECUTABLE_SIZE {
        return Err(Error::NoExec);
    }
    let mut image = Vec::new();
    image
        .try_reserve_exact(image_length)
        .map_err(|_| Error::NoMem)?;
    image.resize(image_length, 0);
    let mut image_offset = 0;
    while image_offset < image_length {
        let count = image_file.read_blocking(&mut image[image_offset..])?;
        if count == 0 || count > image_length - image_offset {
            return Err(Error::Io);
        }
        image_offset += count;
    }
    drop(image_file);

    let source = SliceFile::new(&image);
    let mut address_space = UserAddressSpace::new()?;
    let mut loaded_object = LoadedObject::load(
        &source,
        &mut address_space,
        ElfKind::Executable,
        None,
    )?;
    loaded_object.prepare_for_self_relocation()?;
    loaded_object.seal(&mut address_space)?;
    let entry = loaded_object.entry().ok_or(Error::NoExec)?;
    let load_bias = usize::try_from(loaded_object.load_bias()).map_err(|_| Error::Overflow)?;
    let (program_headers, program_header_entry_size, program_header_count) =
        program_header_address(&image, load_bias)?;

    let stack_base = address_space
        .reserve_user_mapping(None, DEFAULT_STACK_SIZE, 2, false)
        .ok_or(Error::NoMem)?;
    let stack_pointer = build_initial_stack(
        &mut address_space,
        stack_base,
        DEFAULT_STACK_SIZE,
        entry,
        program_headers,
        program_header_entry_size,
        program_header_count,
        &process.credentials()?,
    )?;
    crate::kernel::scheduler::thread::ThreadManager::exec_current(
        address_space.clone(),
        entry,
        stack_pointer,
    )?;
    unsafe { address_space.enter(&loaded_object, stack_pointer) }
}

fn program_header_address(
    image: &[u8],
    load_bias: usize,
) -> Result<(usize, usize, usize), Error> {
    if image.len() < 64 || &image[..4] != b"\x7fELF" || image[4] != 2 || image[5] != 1 {
        return Err(Error::NoExec);
    }
    let file_offset = usize::try_from(read_u64(image, 32)?).map_err(|_| Error::Overflow)?;
    let entry_size = usize::from(read_u16(image, 54)?);
    let entry_count = usize::from(read_u16(image, 56)?);
    if entry_size < 56 || entry_count == 0 {
        return Err(Error::NoExec);
    }
    let table_size = entry_size
        .checked_mul(entry_count)
        .ok_or(Error::Overflow)?;
    let table_end = file_offset.checked_add(table_size).ok_or(Error::Overflow)?;
    if table_end > image.len() {
        return Err(Error::NoExec);
    }

    for index in 0..entry_count {
        let header = file_offset
            .checked_add(index.checked_mul(entry_size).ok_or(Error::Overflow)?)
            .ok_or(Error::Overflow)?;
        let kind = read_u32(image, header)?;
        let file_start = usize::try_from(read_u64(image, header + 8)?).map_err(|_| Error::Overflow)?;
        let virtual_start = usize::try_from(read_u64(image, header + 16)?).map_err(|_| Error::Overflow)?;
        if kind == PT_PHDR {
            return Ok((
                load_bias.checked_add(virtual_start).ok_or(Error::Overflow)?,
                entry_size,
                entry_count,
            ));
        }
        if kind == PT_LOAD {
            let file_size = usize::try_from(read_u64(image, header + 32)?).map_err(|_| Error::Overflow)?;
            let load_file_end = file_start.checked_add(file_size).ok_or(Error::Overflow)?;
            if file_offset >= file_start && table_end <= load_file_end {
                let offset = file_offset.checked_sub(file_start).ok_or(Error::Overflow)?;
                let address = load_bias
                    .checked_add(virtual_start)
                    .and_then(|start| start.checked_add(offset))
                    .ok_or(Error::Overflow)?;
                return Ok((address, entry_size, entry_count));
            }
        }
    }
    Err(Error::NoExec)
}

fn build_initial_stack(
    address_space: &mut UserAddressSpace,
    stack_base: usize,
    stack_size: usize,
    entry: usize,
    program_headers: usize,
    program_header_entry_size: usize,
    program_header_count: usize,
    credentials: &crate::kernel::process::Credentials,
) -> Result<usize, Error> {
    let arguments: [&[u8]; 2] = [b"/fs/busybox", b"sh"];
    let environment: [&[u8]; 3] = [b"PATH=/fs:/bin:/sbin", b"HOME=/", b"TERM=xterm"];
    let platform = b"x86_64\0";
    let executable = b"/fs/busybox\0";
    let mut stack_pointer = stack_base.checked_add(stack_size).ok_or(Error::Overflow)?;

    let platform_pointer = write_stack_bytes(address_space, &mut stack_pointer, platform, 8)?;
    let executable_pointer = write_stack_bytes(address_space, &mut stack_pointer, executable, 8)?;
    let mut random_bytes = [0; 16];
    crate::random::fill(&mut random_bytes)?;
    let random_pointer = write_stack_bytes(address_space, &mut stack_pointer, &random_bytes, 16)?;

    let mut environment_pointers = Vec::new();
    environment_pointers
        .try_reserve_exact(environment.len())
        .map_err(|_| Error::NoMem)?;
    for value in environment.iter().rev() {
        environment_pointers.push(write_stack_string(address_space, &mut stack_pointer, value)?);
    }
    environment_pointers.reverse();

    let mut argument_pointers = Vec::new();
    argument_pointers
        .try_reserve_exact(arguments.len())
        .map_err(|_| Error::NoMem)?;
    for value in arguments.iter().rev() {
        argument_pointers.push(write_stack_string(address_space, &mut stack_pointer, value)?);
    }
    argument_pointers.reverse();

    let mut auxiliary = Vec::new();
    auxiliary.try_reserve_exact(16).map_err(|_| Error::NoMem)?;
    auxiliary.extend([
        (AT_PHDR, program_headers),
        (AT_PHENT, program_header_entry_size),
        (AT_PHNUM, program_header_count),
        (AT_PAGESZ, PAGE_SIZE),
        (AT_BASE, 0),
        (AT_FLAGS, 0),
        (AT_ENTRY, entry),
        (AT_UID, credentials.real_user().raw() as usize),
        (AT_EUID, credentials.effective_user().raw() as usize),
        (AT_GID, credentials.real_group().raw() as usize),
        (AT_EGID, credentials.effective_group().raw() as usize),
        (AT_PLATFORM, platform_pointer),
        (AT_HWCAP, 0),
        (AT_CLKTCK, 100),
        (AT_SECURE, 0),
        (AT_RANDOM, random_pointer),
        (AT_HWCAP2, 0),
        (AT_EXECFN, executable_pointer),
        (AT_NULL, 0),
    ]);
    let word_count = 1usize
        .checked_add(argument_pointers.len())
        .and_then(|count| count.checked_add(1))
        .and_then(|count| count.checked_add(environment_pointers.len()))
        .and_then(|count| count.checked_add(1))
        .and_then(|count| count.checked_add(auxiliary.len().checked_mul(2)?))
        .ok_or(Error::Overflow)?;
    let byte_length = word_count
        .checked_mul(size_of::<usize>())
        .ok_or(Error::Overflow)?;
    stack_pointer = stack_pointer
        .checked_sub(byte_length)
        .ok_or(Error::NoMem)?
        & !15;
    let mut words = Vec::new();
    words.try_reserve_exact(word_count).map_err(|_| Error::NoMem)?;
    words.push(argument_pointers.len());
    words.extend(argument_pointers);
    words.push(0);
    words.extend(environment_pointers);
    words.push(0);
    for (kind, value) in auxiliary {
        words.push(kind);
        words.push(value);
    }
    let bytes = unsafe {
        core::slice::from_raw_parts(words.as_ptr().cast::<u8>(), words.len() * size_of::<usize>())
    };
    address_space.zero(stack_pointer, bytes.len())?;
    address_space.write(stack_pointer, bytes)?;
    Ok(stack_pointer)
}

fn write_stack_string(
    address_space: &mut UserAddressSpace,
    stack_pointer: &mut usize,
    value: &[u8],
) -> Result<usize, Error> {
    let length = value.len().checked_add(1).ok_or(Error::Overflow)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| Error::NoMem)?;
    bytes.extend_from_slice(value);
    bytes.push(0);
    write_stack_bytes(address_space, stack_pointer, &bytes, 8)
}

fn write_stack_bytes(
    address_space: &mut UserAddressSpace,
    stack_pointer: &mut usize,
    bytes: &[u8],
    alignment: usize,
) -> Result<usize, Error> {
    let start = stack_pointer
        .checked_sub(bytes.len())
        .ok_or(Error::NoMem)?
        & !(alignment - 1);
    address_space.zero(start, bytes.len())?;
    address_space.write(start, bytes)?;
    *stack_pointer = start;
    Ok(start)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, Error> {
    let value = bytes.get(offset..offset + 2).ok_or(Error::NoExec)?;
    Ok(u16::from_le_bytes(value.try_into().map_err(|_| Error::NoExec)?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, Error> {
    let value = bytes.get(offset..offset + 4).ok_or(Error::NoExec)?;
    Ok(u32::from_le_bytes(value.try_into().map_err(|_| Error::NoExec)?))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, Error> {
    let value = bytes.get(offset..offset + 8).ok_or(Error::NoExec)?;
    Ok(u64::from_le_bytes(value.try_into().map_err(|_| Error::NoExec)?))
}
