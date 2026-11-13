/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_void};
use core::mem::size_of;

use crate::kernel::dl::{AddressSpace, ElfKind, LoadedObject, SliceFile, SliceFileApi as _, SymbolResolver};
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::scheduler::thread::{ThreadManager, ThreadStack};
use crate::kernel::signal::SignalSet;
use crate::kernel::scheduler::thread::vm::UserAddressSpace;

use super::errno::{as_errno, E2BIG, EFAULT, EINVAL, ENOMEM};

const EACCES: c_int = as_errno(crate::kernel::error::Error::Acces);
const ENOENT: c_int = as_errno(crate::kernel::error::Error::NoEnt);
const ENOEXEC: c_int = as_errno(crate::kernel::error::Error::NoExec);

const MAX_VECTOR_ITEMS: usize = 4096;
const MAX_ARGUMENT_BYTES: usize = 128 * 1024;
const MAX_STRING_BYTES: usize = 4096;
const DEFAULT_USER_STACK_SIZE: usize = 1024 * 1024;
const PAGE_SIZE: usize = 4096;
const AT_NULL: usize = 0;
const AT_PAGESZ: usize = 6;
const AT_ENTRY: usize = 9;
const AT_EXECFN: usize = 31;
const POSIX_SPAWN_RESETIDS: i16 = 0x01;
const POSIX_SPAWN_SETPGROUP: i16 = 0x02;
const POSIX_SPAWN_SETSIGDEF: i16 = 0x04;
const POSIX_SPAWN_SETSIGMASK: i16 = 0x08;
const POSIX_SPAWN_SETSCHEDPARAM: i16 = 0x10;
const POSIX_SPAWN_SETSCHEDULER: i16 = 0x20;
const POSIX_SPAWN_SETSID: i16 = 0x40;

struct EmptySymbolResolver;

impl SymbolResolver for EmptySymbolResolver {
    fn resolve(&mut self, _name: &[u8]) -> Option<crate::kernel::dl::ResolvedSymbol> {
        None
    }
}

struct PreparedImage {
    address_space: UserAddressSpace,
    loaded_object: LoadedObject,
    entry: usize,
    stack_pointer: usize,
}

fn read_guest_string(process: &Process, address: usize) -> Result<Vec<u8>, c_int> {
    if address == 0 {
        return Err(EFAULT);
    }
    let address_space = process.address_space();
    let mut output = Vec::new();
    output.try_reserve(64).map_err(|_| ENOMEM)?;
    for offset in 0..MAX_STRING_BYTES {
        let current = address.checked_add(offset).ok_or(EFAULT)?;
        let mut byte = [0; 1];
        address_space
            .read(current, &mut byte)
            .map_err(as_errno)?;
        if byte[0] == 0 {
            return Ok(output);
        }
        output.try_reserve(1).map_err(|_| ENOMEM)?;
        output.push(byte[0]);
    }
    Err(E2BIG)
}

fn read_guest_vector(
    process: &Process,
    vector: *const *mut c_char,
    total_bytes: &mut usize,
) -> Result<Vec<Vec<u8>>, c_int> {
    let mut strings = Vec::new();
    if vector.is_null() {
        return Ok(strings);
    }
    strings.try_reserve(8).map_err(|_| ENOMEM)?;
    let address_space = process.address_space();
    for index in 0..MAX_VECTOR_ITEMS {
        let item_address = (vector as usize)
            .checked_add(index.checked_mul(size_of::<usize>()).ok_or(E2BIG)?)
            .ok_or(EFAULT)?;
        let mut pointer_bytes = [0; size_of::<usize>()];
        address_space
            .read(item_address, &mut pointer_bytes)
            .map_err(as_errno)?;
        let pointer = usize::from_ne_bytes(pointer_bytes);
        if pointer == 0 {
            return Ok(strings);
        }
        let string = read_guest_string(process, pointer)?;
        *total_bytes = total_bytes
            .checked_add(string.len().saturating_add(1))
            .ok_or(E2BIG)?;
        if *total_bytes > MAX_ARGUMENT_BYTES {
            return Err(E2BIG);
        }
        strings.try_reserve(1).map_err(|_| ENOMEM)?;
        strings.push(string);
    }
    Err(E2BIG)
}

fn write_stack_string(
    address_space: &mut UserAddressSpace,
    stack_pointer: &mut usize,
    value: &[u8],
) -> Result<usize, c_int> {
    let length = value.len().checked_add(1).ok_or(E2BIG)?;
    let start = stack_pointer.checked_sub(length).ok_or(E2BIG)? & !7;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| ENOMEM)?;
    bytes.extend_from_slice(value);
    bytes.push(0);
    address_space.write(start, &bytes).map_err(as_errno)?;
    *stack_pointer = start;
    Ok(start)
}

fn build_initial_stack(
    address_space: &mut UserAddressSpace,
    stack_base: usize,
    stack_size: usize,
    arguments: &[Vec<u8>],
    environment: &[Vec<u8>],
    executable_path: &[u8],
    entry: usize,
) -> Result<usize, c_int> {
    let mut stack_pointer = stack_base.checked_add(stack_size).ok_or(E2BIG)?;
    let executable_name = write_stack_string(address_space, &mut stack_pointer, executable_path)?;
    let mut environment_pointers = Vec::new();
    environment_pointers
        .try_reserve_exact(environment.len())
        .map_err(|_| ENOMEM)?;
    for value in environment.iter().rev() {
        environment_pointers.push(write_stack_string(address_space, &mut stack_pointer, value)?);
    }
    environment_pointers.reverse();
    let mut argument_pointers = Vec::new();
    argument_pointers
        .try_reserve_exact(arguments.len())
        .map_err(|_| ENOMEM)?;
    for value in arguments.iter().rev() {
        argument_pointers.push(write_stack_string(address_space, &mut stack_pointer, value)?);
    }
    argument_pointers.reverse();

    let word_count = 1usize
        .checked_add(argument_pointers.len())
        .and_then(|count| count.checked_add(1))
        .and_then(|count| count.checked_add(environment_pointers.len()))
        .and_then(|count| count.checked_add(1))
        .and_then(|count| count.checked_add(8))
        .ok_or(E2BIG)?;
    let byte_length = word_count.checked_mul(size_of::<usize>()).ok_or(E2BIG)?;
    stack_pointer = stack_pointer.checked_sub(byte_length).ok_or(E2BIG)? & !15;
    let mut words = Vec::new();
    words.try_reserve_exact(word_count).map_err(|_| ENOMEM)?;
    words.push(argument_pointers.len());
    words.extend(argument_pointers);
    words.push(0);
    words.extend(environment_pointers);
    words.push(0);
    words.extend([AT_PAGESZ, PAGE_SIZE, AT_ENTRY, entry, AT_EXECFN, executable_name]);
    words.extend([AT_NULL, 0]);
    let bytes = unsafe {
        core::slice::from_raw_parts(words.as_ptr().cast::<u8>(), words.len() * size_of::<usize>())
    };
    address_space.write(stack_pointer, bytes).map_err(as_errno)?;
    Ok(stack_pointer)
}

fn prepare_image(
    process: &Process,
    executable: &[u8],
    arguments: &[Vec<u8>],
    environment: &[Vec<u8>],
) -> Result<PreparedImage, c_int> {
    let image_bytes = crate::kernel::runtime::runtime_libc::filesystem::executable_snapshot_for_process(
        executable,
        process.pid(),
    )?;
    prepare_image_bytes(process, executable, image_bytes, arguments, environment)
}

fn prepare_image_bytes(
    process: &Process,
    executable: &[u8],
    image_bytes: Vec<u8>,
    arguments: &[Vec<u8>],
    environment: &[Vec<u8>],
) -> Result<PreparedImage, c_int> {
    let source = SliceFile::new(&image_bytes);
    let mut address_space = UserAddressSpace::new().map_err(as_errno)?;
    let mut loaded_object = LoadedObject::load(
        &source,
        &mut address_space,
        ElfKind::Executable,
        None,
    )
    .map_err(as_errno)?;
    let mut resolver = EmptySymbolResolver;
    loaded_object
        .relocate(&mut address_space, &mut resolver)
        .map_err(as_errno)?;
    loaded_object.seal(&mut address_space).map_err(as_errno)?;
    let entry = loaded_object.entry().ok_or(ENOEXEC)?;
    let (stack_limit, _) = process.resource_limit(3).map_err(as_errno)?;
    let mut stack_size = DEFAULT_USER_STACK_SIZE.min(usize::try_from(stack_limit).unwrap_or(usize::MAX));
    stack_size &= !(PAGE_SIZE - 1);
    if stack_size < 64 * 1024 {
        return Err(E2BIG);
    }
    let stack_base = address_space
        .reserve_user_mapping(None, stack_size, 2, false)
        .ok_or(ENOMEM)?;
    let stack_pointer = build_initial_stack(
        &mut address_space,
        stack_base,
        stack_size,
        arguments,
        environment,
        executable,
        entry,
    )?;
    Ok(PreparedImage {
        address_space,
        loaded_object,
        entry,
        stack_pointer,
    })
}

fn read_arguments(
    process: &Process,
    arguments: *const *mut c_char,
    environment: *const *mut c_char,
) -> Result<(Vec<Vec<u8>>, Vec<Vec<u8>>), c_int> {
    let mut total_bytes = 0usize;
    let arguments = read_guest_vector(process, arguments, &mut total_bytes)?;
    let environment = read_guest_vector(process, environment, &mut total_bytes)?;
    Ok((arguments, environment))
}

pub(crate) unsafe fn execve(
    path: *const c_char,
    arguments: *const *mut c_char,
    environment: *const *mut c_char,
) -> c_int {
    unsafe { execvpe(path, arguments, environment, false) }
}

pub(crate) unsafe fn execvpe(
    path: *const c_char,
    arguments: *const *mut c_char,
    environment: *const *mut c_char,
    search_path: bool,
) -> c_int {
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => return fail(as_errno(error)),
    };
    let requested = match read_guest_string(&process, path as usize) {
        Ok(path) => path,
        Err(error) => return fail(error),
    };
    let (arguments, environment) = match read_arguments(&process, arguments, environment) {
        Ok(values) => values,
        Err(error) => return fail(error),
    };
    let executable = match find_executable(&process, &requested, &environment, search_path) {
        Ok(path) => path,
        Err(error) => return fail(error),
    };
    let image = match prepare_image(&process, &executable, &arguments, &environment) {
        Ok(image) => image,
        Err(error) => return fail(error),
    };
    let PreparedImage {
        address_space,
        loaded_object,
        entry,
        stack_pointer,
    } = image;
    if let Err(error) = ThreadManager::exec_current(address_space.clone(), entry, stack_pointer) {
        return fail(as_errno(error));
    }
    let _ = crate::kernel::runtime::runtime_libc::filesystem::close_on_exec_descriptors();
    crate::kernel::runtime::runtime_posix::mqueue::close_on_exec_for_process(process.pid());
    unsafe { address_space.enter(&loaded_object, stack_pointer) }
}

pub(crate) unsafe fn fexecve(
    file_descriptor: c_int,
    arguments: *const *mut c_char,
    environment: *const *mut c_char,
) -> c_int {
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => return fail(as_errno(error)),
    };
    let (arguments, environment) = match read_arguments(&process, arguments, environment) {
        Ok(values) => values,
        Err(error) => return fail(error),
    };
    let image_bytes = match crate::kernel::runtime::runtime_libc::filesystem::executable_descriptor_snapshot(
        file_descriptor,
    ) {
        Ok(bytes) => bytes,
        Err(error) => return fail(error),
    };
    let executable = b"<fd>";
    let image = match prepare_image_bytes(
        &process,
        executable,
        image_bytes,
        &arguments,
        &environment,
    ) {
        Ok(image) => image,
        Err(error) => return fail(error),
    };
    let PreparedImage {
        address_space,
        loaded_object,
        entry,
        stack_pointer,
    } = image;
    if let Err(error) = ThreadManager::exec_current(address_space.clone(), entry, stack_pointer) {
        return fail(as_errno(error));
    }
    let _ = crate::kernel::runtime::runtime_libc::filesystem::close_on_exec_descriptors();
    crate::kernel::runtime::runtime_posix::mqueue::close_on_exec_for_process(process.pid());
    unsafe { address_space.enter(&loaded_object, stack_pointer) }
}

pub(crate) unsafe fn posix_spawn(
    process_id_output: *mut i32,
    path: *const c_char,
    file_actions: *const c_void,
    attributes: *const c_void,
    arguments: *const *mut c_char,
    environment: *const *mut c_char,
    search_path: bool,
) -> c_int {
    if process_id_output.is_null() {
        return EFAULT;
    }
    let file_actions = match super::spawn::file_actions_snapshot(file_actions) {
        Ok(actions) => actions,
        Err(error) => return error,
    };
    if !file_actions.is_empty() {
        return as_errno(Error::OpNotSupp);
    }
    let attributes = match super::spawn::spawn_attributes(attributes) {
        Ok(attributes) => attributes,
        Err(error) => return error,
    };
    if attributes.is_some_and(|attributes| {
        attributes.flags & POSIX_SPAWN_SETSID != 0
            && attributes.flags & POSIX_SPAWN_SETPGROUP != 0
    }) {
        return EINVAL;
    }
    let parent = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => return as_errno(error),
    };
    if !parent
        .address_space()
        .user_range_accessible(process_id_output as usize, size_of::<i32>(), true)
    {
        return EFAULT;
    }
    let requested = match read_guest_string(&parent, path as usize) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let (arguments, environment) = match read_arguments(&parent, arguments, environment) {
        Ok(values) => values,
        Err(error) => return error,
    };
    let mut credentials = match parent.credentials() {
        Ok(credentials) => credentials,
        Err(error) => return as_errno(error),
    };
    if attributes.is_some_and(|attributes| attributes.flags & POSIX_SPAWN_RESETIDS != 0) {
        credentials.reset_effective_ids();
    }
    let address_space = match UserAddressSpace::new() {
        Ok(address_space) => address_space,
        Err(error) => return as_errno(error),
    };
    let child = match Process::new_child_with_address_space(
        &parent,
        parent.namespace_set(),
        credentials,
        address_space,
    ) {
        Ok(child) => child,
        Err(error) => return as_errno(error),
    };
    let child_id = child.pid();
    if let Err(error) = crate::kernel::runtime::runtime_libc::filesystem::inherit_descriptors_for_process(
        parent.pid(),
        child_id,
        &[],
    ) {
        crate::kernel::runtime::runtime_libc::filesystem::cleanup_process_descriptors_for_process(
            child_id,
        );
        parent.discard_unstarted_child(child_id);
        return error;
    }
    child.inherit_spawn_signal_state(
        &parent,
        attributes
            .filter(|attributes| attributes.flags & POSIX_SPAWN_SETSIGDEF != 0)
            .map_or(SignalSet::EMPTY, |attributes| attributes.signal_default),
    );
    if let Some(attributes) = attributes {
        let operation = (|| -> Result<(), Error> {
            if attributes.flags & POSIX_SPAWN_SETSID != 0 {
                child.create_session()?;
            } else if attributes.flags & POSIX_SPAWN_SETPGROUP != 0 {
                child.set_process_group(child.pid(), attributes.process_group)?;
            }
            if attributes.flags & (POSIX_SPAWN_SETSCHEDPARAM | POSIX_SPAWN_SETSCHEDULER) != 0 {
                let mut scheduling = child.scheduling_configuration();
                if attributes.flags & POSIX_SPAWN_SETSCHEDULER != 0 {
                    scheduling.policy = attributes.scheduling.policy;
                    scheduling.sporadic = attributes.scheduling.sporadic;
                }
                if attributes.flags & POSIX_SPAWN_SETSCHEDPARAM != 0 {
                    scheduling.priority = attributes.scheduling.priority;
                    if scheduling.policy == attributes.scheduling.policy {
                        scheduling.sporadic = attributes.scheduling.sporadic;
                    }
                }
                crate::kernel::runtime::runtime_posix::sched::validate_scheduling_configuration(
                    &parent,
                    scheduling,
                )?;
                child.set_scheduling_configuration(scheduling);
            }
            Ok(())
        })();
        if let Err(error) = operation {
            crate::kernel::runtime::runtime_libc::filesystem::cleanup_process_descriptors_for_process(
                child_id,
            );
            parent.discard_unstarted_child(child_id);
            return as_errno(error);
        }
    }
    let candidate = match find_executable(&child, &requested, &environment, search_path) {
        Ok(path) => path,
        Err(error) => {
            crate::kernel::runtime::runtime_libc::filesystem::cleanup_process_descriptors_for_process(
                child_id,
            );
            parent.discard_unstarted_child(child_id);
            return error;
        }
    };
    let image = match prepare_image(&child, &candidate, &arguments, &environment) {
        Ok(image) => image,
        Err(error) => {
            crate::kernel::runtime::runtime_libc::filesystem::cleanup_process_descriptors_for_process(
                child_id,
            );
            parent.discard_unstarted_child(child_id);
            return error;
        }
    };
    child.replace_address_space(image.address_space.clone());
    if let Err(error) = crate::kernel::runtime::runtime_libc::filesystem::close_on_exec_descriptors_for_process(
        child_id,
    ) {
        crate::kernel::runtime::runtime_libc::filesystem::cleanup_process_descriptors_for_process(
            child_id,
        );
        parent.discard_unstarted_child(child_id);
        return error;
    }
    crate::kernel::runtime::runtime_libc::fd_alias::release_reserved_targets_for_process(child_id);
    let signal_mask = if let Some(attributes) = attributes {
        if attributes.flags & POSIX_SPAWN_SETSIGMASK != 0 {
            attributes.signal_mask
        } else {
            ThreadManager::current_signal_context()
                .map(|(_, mask)| mask)
                .unwrap_or(SignalSet::EMPTY)
        }
    } else {
        ThreadManager::current_signal_context()
            .map(|(_, mask)| mask)
            .unwrap_or(SignalSet::EMPTY)
    };
    if let Err(error) = ThreadManager::create_userspace_thread_in_process_with_signal_mask(
        Arc::clone(&child),
        ThreadStack::DEFAULT_SIZE,
        image.entry,
        image.stack_pointer,
        false,
        signal_mask,
    ) {
        crate::kernel::runtime::runtime_libc::filesystem::cleanup_process_descriptors_for_process(
            child_id,
        );
        parent.discard_unstarted_child(child_id);
        return as_errno(error);
    }
    let pid_bytes = unsafe {
        core::slice::from_raw_parts((&child_id as *const i32).cast::<u8>(), size_of::<i32>())
    };
    if let Err(error) = parent
        .address_space()
        .write(process_id_output as usize, pid_bytes)
    {
        child.terminate_by_signal(crate::kernel::signal::SIGKILL);
        return as_errno(error);
    }
    0
}

fn find_executable(
    process: &Process,
    requested: &[u8],
    environment: &[Vec<u8>],
    search_path: bool,
) -> Result<Vec<u8>, c_int> {
    if !search_path || requested.contains(&b'/') {
        return Ok(requested.to_vec());
    }
    let search = environment
        .iter()
        .find_map(|value| value.strip_prefix(b"PATH="))
        .unwrap_or(b"/bin:/usr/bin");
    let mut start = 0usize;
    while start <= search.len() {
        let end = search[start..]
            .iter()
            .position(|byte| *byte == b':')
            .map_or(search.len(), |offset| start + offset);
        let directory = &search[start..end];
        let directory = if directory.is_empty() { b".".as_slice() } else { directory };
        let mut candidate = Vec::new();
        let needed = directory
            .len()
            .checked_add(1)
            .and_then(|length| length.checked_add(requested.len()))
            .ok_or(E2BIG)?;
        candidate.try_reserve_exact(needed).map_err(|_| ENOMEM)?;
        candidate.extend_from_slice(directory);
        if !candidate.ends_with(b"/") {
            candidate.push(b'/');
        }
        candidate.extend_from_slice(requested);
        match crate::kernel::runtime::runtime_libc::filesystem::executable_snapshot_for_process(
            &candidate,
            process.pid(),
        ) {
            Ok(_) => return Ok(candidate),
            Err(error) if error == ENOENT => {}
            Err(error) => return Err(error),
        }
        if end == search.len() {
            break;
        }
        start = end + 1;
    }
    Err(ENOENT)
}

fn fail(error: c_int) -> c_int {
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
    -1
}
