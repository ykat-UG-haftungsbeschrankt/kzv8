/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_void};
use core::mem::{size_of, MaybeUninit};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::kernel::dl::{
    AddressSpace, ElfKind, LoadedObject, SliceFile, SliceFileApi as _, SymbolScope,
    SymbolScopeApi as _,
};
use crate::kernel::error::Error;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::scheduler::thread::{vm::UserAddressSpace, ThreadManager};

use super::errno::{as_errno, EINVAL, ENOSYS};

const O_RDONLY: c_int = 0;
const PATH_MAX: usize = 4096;
const MAX_SHARED_OBJECT_SIZE: usize = 1 << 28;
const RTLD_LAZY: c_int = 0x0001;
const RTLD_NOW: c_int = 0x0002;
const RTLD_GLOBAL: c_int = 0x0100;
const HANDLE_SLOT_BITS: usize = 8;
const HANDLE_SLOT_MASK: usize = (1 << HANDLE_SLOT_BITS) - 1;
const HANDLE_GENERATION_MASK: usize = usize::MAX >> HANDLE_SLOT_BITS;

static ERROR_PENDING: AtomicBool = AtomicBool::new(false);
static ERROR_MESSAGE: AtomicUsize = AtomicUsize::new(0);

const MESSAGE_ADDRESS: &[u8] = b"dynamic address lookup is not supported\0";
const MESSAGE_ARGUMENT: &[u8] = b"invalid dynamic loader argument\0";
const MESSAGE_FILE: &[u8] = b"unable to load shared object\0";
const MESSAGE_OBJECT: &[u8] = b"invalid or unsupported shared object\0";
const MESSAGE_SYMBOL: &[u8] = b"dynamic symbol not found\0";
const MESSAGE_MEMORY: &[u8] = b"dynamic loader resource limit exceeded\0";

#[derive(Clone, Copy)]
struct DlFailure {
    errno: c_int,
    message: &'static [u8],
}

impl DlFailure {
    fn from_kernel(error: Error) -> Self {
        Self {
            errno: as_errno(error),
            message: match error {
                Error::NoEnt => MESSAGE_FILE,
                Error::NoMem | Error::Fbig => MESSAGE_MEMORY,
                _ => MESSAGE_OBJECT,
            },
        }
    }

    const fn new(errno: c_int, message: &'static [u8]) -> Self {
        Self { errno, message }
    }
}

struct LoadedLibrary {
    path: Vec<u8>,
    dependencies: Vec<Vec<u8>>,
    object: LoadedObject,
    global: bool,
}

struct StagedLibrary {
    path: Vec<u8>,
    dependencies: Vec<Vec<u8>>,
    object: LoadedObject,
}

struct LibraryHandle {
    generation: usize,
    root: Option<Vec<u8>>,
}

struct ProcessLibraries {
    pid: i32,
    libraries: Vec<LoadedLibrary>,
    handles: Vec<LibraryHandle>,
}

struct LoaderState {
    processes: Vec<ProcessLibraries>,
}

impl LoaderState {
    const fn new() -> Self {
        Self {
            processes: Vec::new(),
        }
    }
}

struct PendingLibrary {
    path: Vec<u8>,
    bytes: Option<Vec<u8>>,
}

static LOADERS: IrqSpinLock<LoaderState> = IrqSpinLock::new(LoaderState::new());

unsafe extern "C" {
    fn kz_rust_open(path: *const c_char, flags: c_int, mode: u32) -> c_int;
    fn read(file_descriptor: c_int, buffer: *mut c_void, length: usize) -> isize;
    fn close(file_descriptor: c_int) -> c_int;
}

#[repr(C)]
pub struct DlInfo {
    file_name: *const c_char,
    file_base: *mut c_void,
    symbol_name: *const c_char,
    symbol_address: *mut c_void,
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

fn set_loader_error(error: DlFailure) {
    ERROR_MESSAGE.store(error.message.as_ptr() as usize, Ordering::Relaxed);
    ERROR_PENDING.store(true, Ordering::Release);
    set_errno(error.errno);
}

fn copy_bytes(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let mut copy = Vec::new();
    copy.try_reserve_exact(bytes.len()).map_err(|_| Error::NoMem)?;
    copy.extend_from_slice(bytes);
    Ok(copy)
}

fn copy_strings(strings: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, Error> {
    let mut copies = Vec::new();
    copies
        .try_reserve_exact(strings.len())
        .map_err(|_| Error::NoMem)?;
    for string in strings {
        copies.push(copy_bytes(string)?);
    }
    Ok(copies)
}

fn read_user_value<T: Copy>(address_space: &UserAddressSpace, pointer: *const T) -> Result<T, Error> {
    if pointer.is_null() {
        return Err(Error::Fault);
    }
    let mut value = MaybeUninit::<T>::uninit();
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), size_of::<T>())
    };
    address_space.read(pointer as usize, bytes)?;
    Ok(unsafe { value.assume_init() })
}

fn read_user_string(
    address_space: &UserAddressSpace,
    pointer: *const c_char,
) -> Result<Vec<u8>, Error> {
    if pointer.is_null() {
        return Err(Error::Fault);
    }
    let start = pointer as usize;
    let mut value = Vec::new();
    for offset in 0..PATH_MAX {
        let address = start.checked_add(offset).ok_or(Error::Fault)?;
        let mut byte = [0];
        address_space.read(address, &mut byte)?;
        if byte[0] == 0 {
            return Ok(value);
        }
        value.try_reserve(1).map_err(|_| Error::NoMem)?;
        value.push(byte[0]);
    }
    Err(Error::NameTooLong)
}

fn normalize_path(path: &[u8], current_directory: &[u8]) -> Result<Vec<u8>, Error> {
    if path.is_empty() {
        return Err(Error::NoEnt);
    }
    let base = if path.first() == Some(&b'/') {
        b"/".as_slice()
    } else {
        current_directory
    };
    let mut normalized = Vec::new();
    normalized
        .try_reserve(base.len().saturating_add(path.len()).saturating_add(1))
        .map_err(|_| Error::NoMem)?;
    normalized.extend_from_slice(base);
    for segment in path.split(|byte| *byte == b'/') {
        if segment.is_empty() || segment == b"." {
            continue;
        }
        if segment == b".." {
            if normalized.len() > 1 {
                let separator = normalized.iter().rposition(|byte| *byte == b'/').unwrap_or(0);
                normalized.truncate(separator.max(1));
            }
            continue;
        }
        let separator = usize::from(normalized.len() > 1);
        if normalized.len() + separator + segment.len() > PATH_MAX {
            return Err(Error::NameTooLong);
        }
        if separator != 0 {
            normalized.push(b'/');
        }
        normalized.extend_from_slice(segment);
    }
    Ok(normalized)
}

fn current_errno() -> c_int {
    unsafe { crate::kernel::scheduler::thread::__errno_location().read() }
}

fn read_library_file(path: &[u8]) -> Result<Vec<u8>, DlFailure> {
    let mut c_path = Vec::new();
    c_path
        .try_reserve_exact(path.len() + 1)
        .map_err(|_| DlFailure::from_kernel(Error::NoMem))?;
    c_path.extend_from_slice(path);
    c_path.push(0);

    let file_descriptor = unsafe { kz_rust_open(c_path.as_ptr().cast(), O_RDONLY, 0) };
    if file_descriptor < 0 {
        let errno = current_errno();
        return Err(DlFailure::new(
            if errno == 0 { as_errno(Error::NoEnt) } else { errno },
            MESSAGE_FILE,
        ));
    }

    let mut contents = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let count = unsafe {
            read(
                file_descriptor,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        if count < 0 {
            let errno = current_errno();
            let _ = unsafe { close(file_descriptor) };
            return Err(DlFailure::new(errno, MESSAGE_FILE));
        }
        if count == 0 {
            break;
        }
        let count = count as usize;
        if contents
            .len()
            .checked_add(count)
            .is_none_or(|length| length > MAX_SHARED_OBJECT_SIZE)
        {
            let _ = unsafe { close(file_descriptor) };
            return Err(DlFailure::from_kernel(Error::Fbig));
        }
        if contents.try_reserve(count).is_err() {
            let _ = unsafe { close(file_descriptor) };
            return Err(DlFailure::from_kernel(Error::NoMem));
        }
        contents.extend_from_slice(&buffer[..count]);
    }
    if unsafe { close(file_descriptor) } != 0 {
        return Err(DlFailure::new(current_errno(), MESSAGE_FILE));
    }
    Ok(contents)
}

fn library_dependencies(pid: i32, path: &[u8]) -> Option<Vec<Vec<u8>>> {
    let loaders = LOADERS.lock();
    let process = loaders.processes.iter().find(|process| process.pid == pid)?;
    let library = process.libraries.iter().find(|library| library.path == path)?;
    Some(library.dependencies.clone())
}

fn path_is_pending(pending: &[PendingLibrary], path: &[u8]) -> bool {
    pending.iter().any(|library| library.path == path)
}

fn path_is_staged(staged: &[StagedLibrary], path: &[u8]) -> bool {
    staged.iter().any(|library| library.path == path)
}

fn join_path(directory: &[u8], name: &[u8]) -> Result<Vec<u8>, Error> {
    let capacity = directory
        .len()
        .checked_add(name.len())
        .and_then(|length| length.checked_add(1))
        .ok_or(Error::Overflow)?;
    let mut joined = Vec::new();
    joined.try_reserve_exact(capacity).map_err(|_| Error::NoMem)?;
    joined.extend_from_slice(directory);
    if directory != b"/" {
        joined.push(b'/');
    }
    joined.extend_from_slice(name);
    normalize_path(&joined, b"/")
}

fn dependency_candidates(parent: &[u8], name: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    let mut candidates = Vec::new();
    candidates.try_reserve(5).map_err(|_| Error::NoMem)?;
    if name.contains(&b'/') {
        if name.first() == Some(&b'/') {
            candidates.push(normalize_path(name, b"/")?);
        } else {
            let directory_end = parent.iter().rposition(|byte| *byte == b'/').unwrap_or(0);
            let directory = if directory_end == 0 {
                b"/".as_slice()
            } else {
                &parent[..directory_end]
            };
            candidates.push(join_path(directory, name)?);
        }
        return Ok(candidates);
    }

    let directory_end = parent.iter().rposition(|byte| *byte == b'/').unwrap_or(0);
    let directory = if directory_end == 0 {
        b"/".as_slice()
    } else {
        &parent[..directory_end]
    };
    for search_directory in [directory, b"/lib", b"/usr/lib", b"/lib64", b"/usr/lib64"] {
        let candidate = join_path(search_directory, name)?;
        if !candidates.iter().any(|existing| *existing == candidate) {
            candidates.push(candidate);
        }
    }
    Ok(candidates)
}

fn resolve_dependency(
    pid: i32,
    parent: &[u8],
    name: &[u8],
    pending: &[PendingLibrary],
    staged: &[StagedLibrary],
) -> Result<(Vec<u8>, Option<Vec<u8>>), DlFailure> {
    let candidates = dependency_candidates(parent, name).map_err(DlFailure::from_kernel)?;
    for candidate in candidates {
        if path_is_pending(pending, &candidate)
            || path_is_staged(staged, &candidate)
            || library_dependencies(pid, &candidate).is_some()
        {
            return Ok((candidate, None));
        }
        match read_library_file(&candidate) {
            Ok(contents) => return Ok((candidate, Some(contents))),
            Err(error) if error.errno == as_errno(Error::NoEnt) => {}
            Err(error) => return Err(error),
        }
    }
    Err(DlFailure::from_kernel(Error::NoEnt))
}

fn release_staged(address_space: &mut UserAddressSpace, staged: Vec<StagedLibrary>) {
    for library in staged {
        library.object.release(address_space);
    }
}

fn stage_load_graph(
    pid: i32,
    root: &[u8],
    address_space: &mut UserAddressSpace,
) -> Result<Vec<StagedLibrary>, DlFailure> {
    let mut pending = Vec::new();
    pending
        .try_reserve(1)
        .map_err(|_| DlFailure::from_kernel(Error::NoMem))?;
    pending.push(PendingLibrary {
        path: copy_bytes(root).map_err(DlFailure::from_kernel)?,
        bytes: None,
    });
    let mut visited = Vec::<Vec<u8>>::new();
    let mut staged = Vec::<StagedLibrary>::new();

    let result = (|| {
        while let Some(current) = pending.pop() {
            if visited.iter().any(|path| *path == current.path) {
                continue;
            }
            visited
                .try_reserve(1)
                .map_err(|_| DlFailure::from_kernel(Error::NoMem))?;
            visited.push(copy_bytes(&current.path).map_err(DlFailure::from_kernel)?);

            if let Some(dependencies) = library_dependencies(pid, &current.path) {
                for dependency in dependencies {
                    if !visited.iter().any(|path| *path == dependency)
                        && !path_is_pending(&pending, &dependency)
                    {
                        pending
                            .try_reserve(1)
                            .map_err(|_| DlFailure::from_kernel(Error::NoMem))?;
                        pending.push(PendingLibrary {
                            path: dependency,
                            bytes: None,
                        });
                    }
                }
                continue;
            }

            let path = current.path;
            let staged_path = copy_bytes(&path).map_err(DlFailure::from_kernel)?;
            staged
                .try_reserve(1)
                .map_err(|_| DlFailure::from_kernel(Error::NoMem))?;
            let contents = match current.bytes {
                Some(contents) => contents,
                None => read_library_file(&path)?,
            };
            let source = SliceFile::new(&contents);
            let object = LoadedObject::load(
                &source,
                address_space,
                ElfKind::SharedObject,
                None,
            )
            .map_err(DlFailure::from_kernel)?;
            let staged_index = staged.len();
            staged.push(StagedLibrary {
                path: staged_path,
                dependencies: Vec::new(),
                object,
            });
            let needed = copy_strings(staged[staged_index].object.needed())
                .map_err(DlFailure::from_kernel)?;
            staged[staged_index]
                .dependencies
                .try_reserve(needed.len())
                .map_err(|_| DlFailure::from_kernel(Error::NoMem))?;

            for name in needed {
                let (dependency, contents) =
                    resolve_dependency(pid, &path, &name, &pending, &staged)?;
                if !staged[staged_index]
                    .dependencies
                    .iter()
                    .any(|existing| *existing == dependency)
                {
                    staged[staged_index]
                        .dependencies
                        .push(copy_bytes(&dependency).map_err(DlFailure::from_kernel)?);
                }
                if contents.is_some()
                    && !visited.iter().any(|existing| *existing == dependency)
                    && !path_is_pending(&pending, &dependency)
                    && !path_is_staged(&staged, &dependency)
                {
                    pending
                        .try_reserve(1)
                        .map_err(|_| DlFailure::from_kernel(Error::NoMem))?;
                    pending.push(PendingLibrary {
                        path: dependency,
                        bytes: contents,
                    });
                } else if contents.is_none()
                    && library_dependencies(pid, &dependency).is_some()
                    && !visited.iter().any(|existing| *existing == dependency)
                    && !path_is_pending(&pending, &dependency)
                {
                    pending
                        .try_reserve(1)
                        .map_err(|_| DlFailure::from_kernel(Error::NoMem))?;
                    pending.push(PendingLibrary {
                        path: dependency,
                        bytes: None,
                    });
                }
            }
        }
        Ok::<(), DlFailure>(())
    })();

    if let Err(error) = result {
        release_staged(address_space, staged);
        return Err(error);
    }
    Ok(staged)
}

fn dependency_closure(
    loader: Option<&ProcessLibraries>,
    staged: &[StagedLibrary],
    roots: &[Vec<u8>],
) -> Vec<Vec<u8>> {
    let mut pending = roots.to_vec();
    let mut closure = Vec::new();
    while let Some(path) = pending.pop() {
        if closure.iter().any(|existing| *existing == path) {
            continue;
        }
        closure.push(path.clone());
        let dependencies = staged
            .iter()
            .find(|library| library.path == path)
            .map(|library| library.dependencies.as_slice())
            .or_else(|| {
                loader
                    .and_then(|loader| loader.libraries.iter().find(|library| library.path == path))
                    .map(|library| library.dependencies.as_slice())
            });
        if let Some(dependencies) = dependencies {
            pending.extend(dependencies.iter().cloned());
        }
    }
    closure
}

fn ensure_process_libraries(
    state: &mut LoaderState,
    pid: i32,
) -> Result<&mut ProcessLibraries, Error> {
    if let Some(index) = state.processes.iter().position(|process| process.pid == pid) {
        return Ok(&mut state.processes[index]);
    }
    state.processes.try_reserve(1).map_err(|_| Error::NoMem)?;
    state.processes.push(ProcessLibraries {
        pid,
        libraries: Vec::new(),
        handles: Vec::new(),
    });
    Ok(state.processes.last_mut().unwrap())
}

fn add_handle(loader: &mut ProcessLibraries, root: &[u8]) -> Result<usize, DlFailure> {
    let root = copy_bytes(root).map_err(DlFailure::from_kernel)?;
    let slot = if let Some(slot) = loader.handles.iter().position(|handle| {
        handle.root.is_none() && handle.generation < HANDLE_GENERATION_MASK
    }) {
        slot
    } else {
        if loader.handles.len() >= HANDLE_SLOT_MASK {
            return Err(DlFailure::from_kernel(Error::NoSpc));
        }
        loader
            .handles
            .try_reserve(1)
            .map_err(|_| DlFailure::from_kernel(Error::NoMem))?;
        loader.handles.push(LibraryHandle {
            generation: 0,
            root: None,
        });
        loader.handles.len() - 1
    };
    let generation = loader.handles[slot].generation + 1;
    let token = (generation << HANDLE_SLOT_BITS) | (slot + 1);
    loader.handles[slot].generation = generation;
    loader.handles[slot].root = Some(root);
    Ok(token)
}

fn handle_parts(token: usize) -> Option<(usize, usize)> {
    let slot = token & HANDLE_SLOT_MASK;
    let generation = token >> HANDLE_SLOT_BITS;
    if slot == 0 || generation == 0 {
        return None;
    }
    Some((slot - 1, generation))
}

fn find_handle(loader: &ProcessLibraries, token: usize) -> Option<usize> {
    let (slot, generation) = handle_parts(token)?;
    let handle = loader.handles.get(slot)?;
    (handle.generation == generation && handle.root.is_some()).then_some(slot)
}

fn invalidate_handle(loader: &mut ProcessLibraries, token: usize) {
    if let Some(slot) = find_handle(loader, token) {
        loader.handles[slot].root = None;
    }
}

fn find_process_libraries(state: &LoaderState, pid: i32) -> Option<&ProcessLibraries> {
    state.processes.iter().find(|process| process.pid == pid)
}

fn mark_global(loader: &mut ProcessLibraries, closure: &[Vec<u8>]) {
    for library in &mut loader.libraries {
        if closure.iter().any(|path| *path == library.path) {
            library.global = true;
        }
    }
}

fn open_existing(
    pid: i32,
    root: &[u8],
    global: bool,
) -> Result<Option<usize>, DlFailure> {
    let mut state = LOADERS.lock();
    let Some(loader_index) = state.processes.iter().position(|process| process.pid == pid) else {
        return Ok(None);
    };
    if !state.processes[loader_index]
        .libraries
        .iter()
        .any(|library| library.path == root)
    {
        return Ok(None);
    }
    let roots = [copy_bytes(root).map_err(DlFailure::from_kernel)?];
    let closure = dependency_closure(Some(&state.processes[loader_index]), &[], &roots);
    let loader = &mut state.processes[loader_index];
    let token = add_handle(loader, root)?;
    if global {
        mark_global(loader, &closure);
    }
    Ok(Some(token))
}

fn commit_staged(
    pid: i32,
    root: &[u8],
    global: bool,
    mut staged: Vec<StagedLibrary>,
    address_space: &mut UserAddressSpace,
) -> Result<usize, DlFailure> {
    let mut state = LOADERS.lock();
    let loader = match ensure_process_libraries(&mut state, pid) {
        Ok(loader) => loader,
        Err(error) => {
            release_staged(address_space, staged);
            return Err(DlFailure::from_kernel(error));
        }
    };
    if loader.libraries.iter().any(|library| library.path == root) {
        release_staged(address_space, staged);
        let roots = [copy_bytes(root).map_err(DlFailure::from_kernel)?];
        let closure = dependency_closure(Some(loader), &[], &roots);
        let token = add_handle(loader, root)?;
        if global {
            mark_global(loader, &closure);
        }
        return Ok(token);
    }

    let mut unique = Vec::new();
    if unique.try_reserve(staged.len()).is_err() {
        release_staged(address_space, staged);
        return Err(DlFailure::from_kernel(Error::NoMem));
    }
    for library in staged.drain(..) {
        if loader
            .libraries
            .iter()
            .any(|existing| existing.path == library.path)
        {
            library.object.release(address_space);
        } else {
            unique.push(library);
        }
    }

    let root_for_closure = match copy_bytes(root) {
        Ok(root) => root,
        Err(error) => {
            release_staged(address_space, unique);
            return Err(DlFailure::from_kernel(error));
        }
    };
    let roots = [root_for_closure];
    let closure = dependency_closure(Some(loader), &unique, &roots);
    let complete = closure.iter().all(|path| {
        unique.iter().any(|library| library.path == *path)
            || loader.libraries.iter().any(|library| library.path == *path)
    });
    if !complete {
        release_staged(address_space, unique);
        return Err(DlFailure::from_kernel(Error::NoEnt));
    }

    let mut reachable = Vec::new();
    if reachable.try_reserve(unique.len()).is_err() {
        release_staged(address_space, unique);
        return Err(DlFailure::from_kernel(Error::NoMem));
    }
    for library in unique {
        if closure.iter().any(|path| *path == library.path) {
            reachable.push(library);
        } else {
            library.object.release(address_space);
        }
    }
    if loader.libraries.try_reserve(reachable.len()).is_err()
        || loader.handles.try_reserve(1).is_err()
    {
        release_staged(address_space, reachable);
        return Err(DlFailure::from_kernel(Error::NoMem));
    }
    let token = match add_handle(loader, root) {
        Ok(token) => token,
        Err(error) => {
            release_staged(address_space, reachable);
            return Err(error);
        }
    };

    for index in 0..reachable.len() {
        let relocation = {
            let (before, current_and_after) = reachable.split_at_mut(index);
            let Some((current, after)) = current_and_after.split_first_mut() else {
                release_staged(address_space, reachable);
                invalidate_handle(loader, token);
                return Err(DlFailure::from_kernel(Error::Inval));
            };
            let mut objects = Vec::new();
            if objects
                .try_reserve(loader.libraries.len() + before.len() + after.len())
                .is_err()
            {
                Err(Error::NoMem)
            } else {
                for library in &loader.libraries {
                    if library.global || closure.iter().any(|path| *path == library.path) {
                        objects.push(&library.object);
                    }
                }
                for library in before.iter().chain(after.iter()) {
                    objects.push(&library.object);
                }
                SymbolScope::new(&objects)
                    .and_then(|mut scope| current.object.relocate(address_space, &mut scope))
            }
        };
        if let Err(error) = relocation {
            release_staged(address_space, reachable);
            invalidate_handle(loader, token);
            return Err(DlFailure::from_kernel(error));
        }
    }
    for library in &mut reachable {
        if let Err(error) = library.object.seal(address_space) {
            release_staged(address_space, reachable);
            invalidate_handle(loader, token);
            return Err(DlFailure::from_kernel(error));
        }
    }

    for library in reachable {
        loader.libraries.push(LoadedLibrary {
            path: library.path,
            dependencies: library.dependencies,
            object: library.object,
            global: false,
        });
    }
    if global {
        mark_global(loader, &closure);
    }
    Ok(token)
}

fn open_library_handle(file: *const c_char, mode: c_int) -> Result<usize, DlFailure> {
    if mode & !(RTLD_LAZY | RTLD_NOW | RTLD_GLOBAL) != 0
        || mode & (RTLD_LAZY | RTLD_NOW) == (RTLD_LAZY | RTLD_NOW)
    {
        return Err(DlFailure::new(EINVAL, MESSAGE_ARGUMENT));
    }
    if file.is_null() {
        return Err(DlFailure::from_kernel(Error::NoEnt));
    }
    let process = ThreadManager::current_process().map_err(DlFailure::from_kernel)?;
    let mut address_space = process.address_space();
    let path = read_user_string(&address_space, file).map_err(DlFailure::from_kernel)?;
    let current_directory = process
        .current_directory()
        .map_err(DlFailure::from_kernel)?;
    let root = normalize_path(&path, &current_directory).map_err(DlFailure::from_kernel)?;
    let global = mode & RTLD_GLOBAL != 0;
    if let Some(token) = open_existing(process.pid(), &root, global)? {
        return Ok(token);
    }
    let staged = stage_load_graph(process.pid(), &root, &mut address_space)?;
    commit_staged(
        process.pid(),
        &root,
        global,
        staged,
        &mut address_space,
    )
}

fn reachable_from_handles(loader: &ProcessLibraries) -> Vec<Vec<u8>> {
    let mut roots = Vec::new();
    roots.extend(loader.handles.iter().filter_map(|handle| handle.root.clone()));
    dependency_closure(Some(loader), &[], &roots)
}

fn lookup_symbol(loader: &ProcessLibraries, root: &[u8], name: &[u8]) -> Option<usize> {
    let roots = [root.to_vec()];
    let closure = dependency_closure(Some(loader), &[], &roots);
    closure.iter().find_map(|path| {
        loader
            .libraries
            .iter()
            .find(|library| library.path == *path)
            .and_then(|library| library.object.symbol(name))
    })
}

fn lookup_global_symbol(loader: &ProcessLibraries, name: &[u8]) -> Option<usize> {
    loader
        .libraries
        .iter()
        .filter(|library| library.global)
        .find_map(|library| library.object.symbol(name))
}

fn current_process_context() -> Result<(i32, UserAddressSpace), DlFailure> {
    let process = ThreadManager::current_process().map_err(DlFailure::from_kernel)?;
    Ok((process.pid(), process.address_space()))
}

fn close_library_handle(handle: *mut c_void) -> Result<(), DlFailure> {
    let token = handle as usize;
    let (pid, mut address_space) = current_process_context()?;
    let mut state = LOADERS.lock();
    let Some(process_index) = state.processes.iter().position(|process| process.pid == pid) else {
        return Err(DlFailure::new(EINVAL, MESSAGE_ARGUMENT));
    };
    let loader = &mut state.processes[process_index];
    let Some(handle_index) = find_handle(loader, token) else {
        return Err(DlFailure::new(EINVAL, MESSAGE_ARGUMENT));
    };
    loader.handles[handle_index].root = None;
    let reachable = reachable_from_handles(loader);
    let mut index = 0;
    while index < loader.libraries.len() {
        if reachable
            .iter()
            .any(|path| *path == loader.libraries[index].path)
        {
            index += 1;
        } else {
            let library = loader.libraries.remove(index);
            library.object.release(&mut address_space);
        }
    }
    if !loader.handles.iter().any(|entry| entry.root.is_some()) && loader.libraries.is_empty() {
        state.processes.remove(process_index);
    }
    Ok(())
}

fn find_library_symbol(handle: *mut c_void, symbol: *const c_char) -> Result<usize, DlFailure> {
    let (pid, address_space) = current_process_context()?;
    let name = read_user_string(&address_space, symbol).map_err(DlFailure::from_kernel)?;
    if name.is_empty() {
        return Err(DlFailure::new(EINVAL, MESSAGE_ARGUMENT));
    }
    let state = LOADERS.lock();
    let Some(loader) = find_process_libraries(&state, pid) else {
        return Err(DlFailure::new(as_errno(Error::NoEnt), MESSAGE_SYMBOL));
    };
    if handle.is_null() {
        return lookup_global_symbol(loader, &name)
            .ok_or_else(|| DlFailure::new(as_errno(Error::NoEnt), MESSAGE_SYMBOL));
    }
    let token = handle as usize;
    let Some(handle_index) = find_handle(loader, token) else {
        return Err(DlFailure::new(EINVAL, MESSAGE_ARGUMENT));
    };
    let Some(root) = loader.handles[handle_index].root.as_deref() else {
        return Err(DlFailure::new(EINVAL, MESSAGE_ARGUMENT));
    };
    lookup_symbol(loader, root, &name)
        .ok_or_else(|| DlFailure::new(as_errno(Error::NoEnt), MESSAGE_SYMBOL))
}

#[no_mangle]
pub unsafe extern "C" fn dladdr(_address: *const c_void, information: *mut DlInfo) -> c_int {
    if !information.is_null() {
        unsafe {
            information.write(DlInfo {
                file_name: core::ptr::null(),
                file_base: core::ptr::null_mut(),
                symbol_name: core::ptr::null(),
                symbol_address: core::ptr::null_mut(),
            });
        }
    }
    set_loader_error(DlFailure::new(ENOSYS, MESSAGE_ADDRESS));
    0
}

#[no_mangle]
pub extern "C" fn dlclose(handle: *mut c_void) -> c_int {
    match close_library_handle(handle) {
        Ok(()) => 0,
        Err(error) => {
            set_loader_error(error);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn dlerror() -> *mut c_char {
    if ERROR_PENDING.swap(false, Ordering::AcqRel) {
        ERROR_MESSAGE.swap(0, Ordering::AcqRel) as *mut c_char
    } else {
        core::ptr::null_mut()
    }
}

#[no_mangle]
pub extern "C" fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void {
    match open_library_handle(file, mode) {
        Ok(handle) => handle as *mut c_void,
        Err(error) => {
            set_loader_error(error);
            core::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "C" fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
    match find_library_symbol(handle, symbol) {
        Ok(address) => address as *mut c_void,
        Err(error) => {
            set_loader_error(error);
            core::ptr::null_mut()
        }
    }
}