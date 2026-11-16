/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::string::String;
use alloc::vec::Vec;
use core::mem::size_of;

use crate::kernel::dl::{
    AddressSpace, ElfKind, LoadedObject, SliceFile, SliceFileApi as _, SymbolScope,
    SymbolScopeApi as _,
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
const PT_INTERP: u32 = 3;
const PT_PHDR: u32 = 6;
const MAX_NATIVE_DEPENDENCIES: usize = 128;

struct NativeDependency {
    aliases: Vec<Vec<u8>>,
    path: String,
    object: LoadedObject,
}

struct PendingDependency {
    name: Vec<u8>,
    owner_path: String,
    search_paths: Vec<Vec<u8>>,
}

pub(crate) fn launch_busybox() -> Result<(), Error> {
    let environment = [
        b"PATH=/fs:/bin:/sbin".as_slice(),
        b"HOME=/".as_slice(),
        b"TERM=xterm".as_slice(),
        b"LD_LIBRARY_PATH=/fs/usr/lib/x86_64-linux-gnu:/fs/lib/x86_64-linux-gnu".as_slice(),
    ];
    if let Some(executable) = option_env!("KZ_BOOT_EXECUTABLE") {
        if !executable.is_empty() {
            let arguments = [executable.as_bytes()];
            return exec_image(executable, arguments.as_slice(), environment.as_slice());
        }
    }
    let arguments = [b"/fs/busybox".as_slice(), b"sh".as_slice()];
    exec_image("/fs/busybox", arguments.as_slice(), environment.as_slice())
}

pub(crate) fn exec_image(
    executable: &str,
    arguments: &[&[u8]],
    environment: &[&[u8]],
) -> Result<(), Error> {
    let process = crate::kernel::scheduler::thread::ThreadManager::current_process()?;
    let credentials = process.credentials()?;
    crate::kernel::debug::debug_fmt(format_args!(
        "kz: exec image start pid={} path={}\n",
        process.pid(), executable
    ));
    let image = read_image(executable, &process)?;
    crate::kernel::debug::debug_fmt(format_args!(
        "kz: exec image read pid={} bytes={}\n",
        process.pid(), image.len()
    ));
    let is_elf = image.starts_with(b"\x7fELF");
    if is_elf {
        if let Some(interpreter) = program_interpreter(&image)? {
            return exec_dynamic_image(
                executable,
                &image,
                interpreter,
                arguments,
                environment,
                &process,
            );
        }
    }

    let mut address_space = UserAddressSpace::new()?;
    let source = SliceFile::new(&image);
    let mut loaded_object = LoadedObject::load(
        &source,
        &mut address_space,
        ElfKind::Executable,
        None,
    )?;
    let program_break = loaded_object
        .mapping_base()
        .checked_add(loaded_object.mapping_length())
        .ok_or(Error::Overflow)?;
    let is_native = loaded_object.is_pe() || loaded_object.is_mach_o();
    let mut dependencies = if is_native {
        load_native_dependencies(executable, &loaded_object, &process, &mut address_space)?
    } else {
        Vec::new()
    };
    if is_native {
        relocate_native_images(&mut loaded_object, &mut dependencies, &mut address_space)?;
        for dependency in &mut dependencies {
            dependency.object.seal(&mut address_space)?;
        }
    } else {
        loaded_object.prepare_for_self_relocation()?;
    }
    loaded_object.seal(&mut address_space)?;
    let mut program_break = program_break;
    for dependency in &dependencies {
        let end = dependency
            .object
            .mapping_base()
            .checked_add(dependency.object.mapping_length())
            .ok_or(Error::Overflow)?;
        program_break = program_break.max(end);
    }
    address_space.set_program_break_base(program_break);
    let stack_base = address_space
        .reserve_user_stack(DEFAULT_STACK_SIZE, 2)
        .ok_or(Error::NoMem)?;
    let entry = loaded_object.entry().ok_or(Error::NoExec)?;
    let (program_headers, program_header_entry_size, program_header_count) = if is_elf {
        let load_bias = usize::try_from(loaded_object.load_bias()).map_err(|_| Error::Overflow)?;
        program_header_address(&image, load_bias)?
    } else {
        (0, 0, 0)
    };

    let stack_pointer = build_initial_stack(
        &mut address_space,
        stack_base,
        DEFAULT_STACK_SIZE,
        entry,
        0,
        program_headers,
        program_header_entry_size,
        program_header_count,
        executable.as_bytes(),
        arguments,
        environment,
        &credentials,
    )?;
    crate::kernel::scheduler::thread::ThreadManager::exec_current(
        address_space.clone(),
        entry,
        stack_pointer,
    )?;
    unsafe { address_space.enter(&loaded_object, stack_pointer) }
}

fn load_native_dependencies(
    executable_path: &str,
    executable: &LoadedObject,
    process: &alloc::sync::Arc<crate::kernel::process::Process>,
    address_space: &mut UserAddressSpace,
) -> Result<Vec<NativeDependency>, Error> {
    let mut pending = Vec::new();
    let root_search_paths = executable.search_paths().to_vec();
    for name in executable.needed() {
        pending.push(PendingDependency {
            name: name.clone(),
            owner_path: String::from(executable_path),
            search_paths: root_search_paths.clone(),
        });
    }

    let mut dependencies = Vec::new();
    let mut pending_index = 0;
    while pending_index < pending.len() {
        if dependencies.len() >= MAX_NATIVE_DEPENDENCIES {
            return Err(Error::Mfile);
        }
        let requested_name = pending[pending_index].name.clone();
        let owner_path = pending[pending_index].owner_path.clone();
        let search_paths = pending[pending_index].search_paths.clone();
        pending_index += 1;

        let path = resolve_native_dependency_path(
            executable_path,
            &owner_path,
            &requested_name,
            &search_paths,
            executable.is_pe(),
        )?;
        if let Some(existing) = dependencies.iter_mut().find(|dependency: &&mut NativeDependency| {
            dependency.path == path
        }) {
            if !existing.aliases.iter().any(|alias| alias == &requested_name) {
                existing.aliases.push(requested_name);
            }
            continue;
        }

        let image = read_image(&path, process)?;
        let source = SliceFile::new(&image);
        let object = LoadedObject::load(
            &source,
            address_space,
            ElfKind::SharedObject,
            None,
        )?;
        if object.is_pe() != executable.is_pe()
            || object.is_mach_o() != executable.is_mach_o()
        {
            return Err(Error::NoExec);
        }
        let needed = object.needed().to_vec();
        let nested_search_paths = object.search_paths().to_vec();
        let mut aliases = Vec::new();
        aliases.try_reserve_exact(1).map_err(|_| Error::NoMem)?;
        aliases.push(requested_name);
        dependencies.push(NativeDependency {
            aliases,
            path: path.clone(),
            object,
        });
        pending
            .try_reserve(needed.len())
            .map_err(|_| Error::NoMem)?;
        for name in needed {
            pending.push(PendingDependency {
                name,
                owner_path: path.clone(),
                search_paths: nested_search_paths.clone(),
            });
        }
    }
    Ok(dependencies)
}

fn relocate_native_images(
    executable: &mut LoadedObject,
    dependencies: &mut [NativeDependency],
    address_space: &mut UserAddressSpace,
) -> Result<(), Error> {
    for index in 0..dependencies.len() {
        let (before, tail) = dependencies.split_at_mut(index);
        let (current, after) = tail.split_first_mut().ok_or(Error::Proto)?;
        let needed = current.object.needed().to_vec();
        let bindings = dependency_bindings(&needed, before.iter().chain(after.iter()))?;
        let mut resolver = SymbolScope::with_dependencies(&[], &bindings)?;
        current.object.relocate(address_space, &mut resolver)?;
    }

    let needed = executable.needed().to_vec();
    let bindings = dependency_bindings(&needed, dependencies.iter())?;
    let mut resolver = SymbolScope::with_dependencies(&[], &bindings)?;
    executable.relocate(address_space, &mut resolver)
}

fn dependency_bindings<'a>(
    needed: &'a [Vec<u8>],
    dependencies: impl Iterator<Item = &'a NativeDependency>,
) -> Result<Vec<(&'a [u8], &'a LoadedObject)>, Error> {
    let dependencies: Vec<&'a NativeDependency> = dependencies.collect();
    let mut bindings = Vec::new();
    bindings
        .try_reserve_exact(needed.len())
        .map_err(|_| Error::NoMem)?;
    for name in needed {
        let dependency = dependencies
            .iter()
            .find(|dependency| dependency.aliases.iter().any(|alias| alias == name))
            .ok_or(Error::NoEnt)?;
        bindings.push((name.as_slice(), &dependency.object));
    }
    Ok(bindings)
}

fn resolve_native_dependency_path(
    executable_path: &str,
    owner_path: &str,
    name: &[u8],
    search_paths: &[Vec<u8>],
    is_pe: bool,
) -> Result<String, Error> {
    let name = core::str::from_utf8(name).map_err(|_| Error::NoExec)?;
    if is_pe {
        let mut normalized = String::new();
        normalized
            .try_reserve_exact(name.len())
            .map_err(|_| Error::NoMem)?;
        normalized.extend(name.chars().map(|character| if character == '\\' { '/' } else { character }));
        return if normalized.starts_with('/') {
            guest_absolute_path(&normalized)
        } else {
            join_path(path_directory(owner_path), &normalized)
        };
    }

    if let Some(suffix) = name.strip_prefix("@rpath/") {
        for search_path in search_paths {
            let search_path = core::str::from_utf8(search_path).map_err(|_| Error::NoExec)?;
            if let Some(directory) = expand_macho_path(search_path, executable_path, owner_path)? {
                return join_path(&directory, suffix);
            }
        }
        return Err(Error::NoEnt);
    }
    if name.starts_with("@loader_path/") || name.starts_with("@executable_path/") {
        return expand_macho_path(name, executable_path, owner_path)?.ok_or(Error::NoEnt);
    }
    if name.starts_with('/') {
        return guest_absolute_path(name);
    }
    join_path(path_directory(owner_path), name)
}

fn expand_macho_path(
    path: &str,
    executable_path: &str,
    owner_path: &str,
) -> Result<Option<String>, Error> {
    if let Some(suffix) = path.strip_prefix("@loader_path/") {
        return join_path(path_directory(owner_path), suffix).map(Some);
    }
    if path == "@loader_path" {
        return Ok(Some(String::from(path_directory(owner_path))));
    }
    if let Some(suffix) = path.strip_prefix("@executable_path/") {
        return join_path(path_directory(executable_path), suffix).map(Some);
    }
    if path == "@executable_path" {
        return Ok(Some(String::from(path_directory(executable_path))));
    }
    if path.starts_with('/') {
        return guest_absolute_path(path).map(Some);
    }
    Ok(None)
}

fn path_directory(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(index) => &path[..index],
        None => "/fs",
    }
}

fn join_path(directory: &str, suffix: &str) -> Result<String, Error> {
    let directory = directory.trim_end_matches('/');
    let suffix = suffix.trim_start_matches('/');
    let capacity = directory
        .len()
        .checked_add(1)
        .and_then(|length| length.checked_add(suffix.len()))
        .ok_or(Error::NameTooLong)?;
    if capacity > 4096 {
        return Err(Error::NameTooLong);
    }
    let mut path = String::new();
    path.try_reserve_exact(capacity).map_err(|_| Error::NoMem)?;
    path.push_str(directory);
    path.push('/');
    path.push_str(suffix);
    Ok(path)
}

fn guest_absolute_path(path: &str) -> Result<String, Error> {
    if path == "/fs" || path.starts_with("/fs/") {
        return Ok(String::from(path));
    }
    join_path("/fs", path)
}

fn read_image(
    executable: &str,
    process: &alloc::sync::Arc<crate::kernel::process::Process>,
) -> Result<Vec<u8>, Error> {
    let vfs = crate::kernel::vfs::mounted().ok_or(Error::NoDev)?;
    crate::kernel::debug::debug_fmt(format_args!(
        "kz: image open start pid={} path={}\n",
        process.pid(), executable
    ));
    let mut image_file = match vfs.open_blocking(
        executable,
        OpenMode::ReadOnly,
        process.credentials()?,
    ) {
        Ok(image_file) => image_file,
        Err(error) => {
            crate::kernel::debug::debug_fmt(format_args!(
                "kz: image open failed pid={} path={} error={:?}\n",
                process.pid(), executable, error
            ));
            return Err(error);
        }
    };
    crate::kernel::debug::debug_fmt(format_args!(
        "kz: image opened pid={} path={}\n",
        process.pid(), executable
    ));
    let image_length = usize::try_from(image_file.size_blocking()?).map_err(|_| Error::Fbig)?;
    crate::kernel::debug::debug_fmt(format_args!(
        "kz: image sized pid={} bytes={}\n",
        process.pid(), image_length
    ));
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
        if image_offset == 0 {
            crate::kernel::debug::debug_fmt(format_args!(
                "kz: image first read start pid={} bytes={}\n",
                process.pid(), image_length
            ));
        }
        let count = image_file.read_blocking(&mut image[image_offset..])?;
        if image_offset == 0 {
            crate::kernel::debug::debug_fmt(format_args!(
                "kz: image first read complete pid={} count={}\n",
                process.pid(), count
            ));
        }
        if count == 0 || count > image_length - image_offset {
            return Err(Error::Io);
        }
        image_offset += count;
    }
    drop(image_file);
    crate::kernel::debug::debug_fmt(format_args!(
        "kz: image read complete pid={} bytes={}\n",
        process.pid(), image_length
    ));
    Ok(image)
}

fn exec_dynamic_image(
    executable: &str,
    image: &[u8],
    interpreter: &str,
    arguments: &[&[u8]],
    environment: &[&[u8]],
    process: &alloc::sync::Arc<crate::kernel::process::Process>,
) -> Result<(), Error> {
    let interpreter_path = resolve_interpreter_path(interpreter)?;
    let interpreter_image = read_image(&interpreter_path, process)?;
    let source = SliceFile::new(image);
    let interpreter_source = SliceFile::new(&interpreter_image);
    let mut address_space = UserAddressSpace::new()?;
    let mut interpreter_object = LoadedObject::load_for_dynamic_exec(
        &interpreter_source,
        &mut address_space,
        ElfKind::Executable,
    )?;
    let mut loaded_object = LoadedObject::load_for_dynamic_exec(
        &source,
        &mut address_space,
        ElfKind::Executable,
    )?;
    let program_break = loaded_object
        .mapping_base()
        .checked_add(loaded_object.mapping_length())
        .ok_or(Error::Overflow)?;
    address_space.set_program_break_base(program_break);
    let stack_base = address_space
        .reserve_user_stack(DEFAULT_STACK_SIZE, 2)
        .ok_or(Error::NoMem)?;
    loaded_object.seal_for_dynamic_exec(&mut address_space)?;
    interpreter_object.seal_for_dynamic_exec(&mut address_space)?;
    let entry = loaded_object.entry().ok_or(Error::NoExec)?;
    let interpreter_entry = interpreter_object.entry().ok_or(Error::NoExec)?;
    let load_bias = usize::try_from(loaded_object.load_bias()).map_err(|_| Error::Overflow)?;
    let (program_headers, program_header_entry_size, program_header_count) =
        program_header_address(image, load_bias)?;

    let stack_pointer = build_initial_stack(
        &mut address_space,
        stack_base,
        DEFAULT_STACK_SIZE,
        entry,
        usize::try_from(interpreter_object.load_bias()).map_err(|_| Error::Overflow)?,
        program_headers,
        program_header_entry_size,
        program_header_count,
        executable.as_bytes(),
        arguments,
        environment,
        &process.credentials()?,
    )?;
    crate::kernel::scheduler::thread::ThreadManager::exec_current(
        address_space.clone(),
        interpreter_entry,
        stack_pointer,
    )?;
    unsafe { address_space.enter(&interpreter_object, stack_pointer) }
}

fn program_interpreter(image: &[u8]) -> Result<Option<&str>, Error> {
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

    let mut interpreter = None;
    for index in 0..entry_count {
        let header = file_offset
            .checked_add(index.checked_mul(entry_size).ok_or(Error::Overflow)?)
            .ok_or(Error::Overflow)?;
        if read_u32(image, header)? != PT_INTERP {
            continue;
        }
        if interpreter.is_some() {
            return Err(Error::NoExec);
        }
        let path_offset = usize::try_from(read_u64(image, header + 8)?)
            .map_err(|_| Error::Overflow)?;
        let path_length = usize::try_from(read_u64(image, header + 32)?)
            .map_err(|_| Error::Overflow)?;
        if path_length < 2 || path_length > 4096 {
            return Err(Error::NoExec);
        }
        let path_end = path_offset.checked_add(path_length).ok_or(Error::Overflow)?;
        let path = image.get(path_offset..path_end).ok_or(Error::NoExec)?;
        if path.last() != Some(&0) || path[..path.len() - 1].contains(&0) {
            return Err(Error::NoExec);
        }
        interpreter = Some(
            core::str::from_utf8(&path[..path.len() - 1]).map_err(|_| Error::NoExec)?,
        );
    }
    Ok(interpreter)
}

fn resolve_interpreter_path(interpreter: &str) -> Result<String, Error> {
    let interpreter_path = if interpreter.starts_with('/') {
        if interpreter == "/fs" || interpreter.starts_with("/fs/") {
            return Ok(interpreter.into());
        }
        interpreter
    } else {
        interpreter
    };
    let capacity = 3usize
        .checked_add(interpreter_path.len())
        .ok_or(Error::NameTooLong)?;
    if capacity > 4096 {
        return Err(Error::NameTooLong);
    }
    let mut resolved = String::new();
    resolved
        .try_reserve_exact(capacity)
        .map_err(|_| Error::NoMem)?;
    resolved.push_str("/fs/");
    resolved.push_str(interpreter_path);
    Ok(resolved)
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
    main_entry: usize,
    interpreter_base: usize,
    program_headers: usize,
    program_header_entry_size: usize,
    program_header_count: usize,
    executable: &[u8],
    arguments: &[&[u8]],
    environment: &[&[u8]],
    credentials: &crate::kernel::process::Credentials,
) -> Result<usize, Error> {
    let platform = b"x86_64\0";
    let mut stack_pointer = stack_base.checked_add(stack_size).ok_or(Error::Overflow)?;

    let platform_pointer = write_stack_bytes(address_space, &mut stack_pointer, platform, 8)?;
    let executable_pointer = write_stack_string(address_space, &mut stack_pointer, executable)?;
    let mut random_bytes = [0; 16];
    crate::random::fill(&mut random_bytes)?;
    let random_pointer = write_stack_bytes(address_space, &mut stack_pointer, &random_bytes, 16)?;

    let mut environment_pointers = Vec::new();
    environment_pointers
        .try_reserve_exact(environment.len())
        .map_err(|_| Error::NoMem)?;
    for value in environment.iter().rev() {
        environment_pointers.push(write_stack_string(
            address_space,
            &mut stack_pointer,
            *value,
        )?);
    }
    environment_pointers.reverse();

    let mut argument_pointers = Vec::new();
    argument_pointers
        .try_reserve_exact(arguments.len())
        .map_err(|_| Error::NoMem)?;
    for value in arguments.iter().rev() {
        argument_pointers.push(write_stack_string(
            address_space,
            &mut stack_pointer,
            *value,
        )?);
    }
    argument_pointers.reverse();

    let mut auxiliary = Vec::new();
    auxiliary.try_reserve_exact(16).map_err(|_| Error::NoMem)?;
    auxiliary.extend([
        (AT_PHDR, program_headers),
        (AT_PHENT, program_header_entry_size),
        (AT_PHNUM, program_header_count),
        (AT_PAGESZ, PAGE_SIZE),
        (AT_BASE, interpreter_base),
        (AT_FLAGS, 0),
        (AT_ENTRY, main_entry),
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
