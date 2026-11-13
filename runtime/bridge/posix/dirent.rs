/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_long, c_void};
use core::mem::{size_of, MaybeUninit};
use core::ptr::read_volatile;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::runtime::runtime_libc::filesystem;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::scheduler::thread::ThreadManager;

use super::errno::{as_errno, EBADF, EFAULT, EINVAL, ENOMEM};

const O_RDONLY: c_int = 0;
const O_DIRECTORY: c_int = 0o200000;
const GETDENTS_SUPPORTED_FLAGS: c_int = 0;
const NAME_MAX: usize = 255;
const DTYPE_UNKNOWN: u8 = 0;
const DTYPE_DIR: u8 = 3;
const DTYPE_REG: u8 = 5;
const DTYPE_LNK: u8 = 6;
const HANDLE_SLOT_BITS: usize = 8;
const HANDLE_SLOT_MASK: usize = (1 << HANDLE_SLOT_BITS) - 1;
const HANDLE_GENERATION_MASK: usize = usize::MAX >> HANDLE_SLOT_BITS;

#[repr(C)]
#[derive(Clone, Copy)]
struct PosixDent {
    inode: u64,
    record_length: u64,
    entry_type: u8,
    name: [c_char; 256],
}

struct DirectoryStream {
    generation: usize,
    owner_pid: i32,
    descriptor: Option<c_int>,
    entries: Vec<crate::kernel::runtime::runtime_libc::filesystem::DirectoryEntry>,
    cursor: usize,
    current: Dirent,
}

static DIRECTORIES: IrqSpinLock<Vec<Box<DirectoryStream>>> = IrqSpinLock::new(Vec::new());
static NEXT_DIRECTORY_GENERATION: AtomicUsize = AtomicUsize::new(1);

unsafe extern "C" {
    fn kz_rust_open(path: *const c_char, flags: c_int, mode: u32) -> c_int;
    fn close(file_descriptor: c_int) -> c_int;
    fn malloc(size: usize) -> *mut c_void;
    fn free(pointer: *mut c_void);
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Dirent {
    inode: u64,
    name: [c_char; 256],
}

fn set_errno(error: c_int) {
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
}

fn current_process() -> Result<alloc::sync::Arc<Process>, c_int> {
    ThreadManager::current_process().map_err(as_errno)
}

fn read_user_value<T: Copy>(process: &Process, pointer: *const T) -> Result<T, Error> {
    if pointer.is_null() {
        return Err(Error::Fault);
    }
    let mut value = MaybeUninit::<T>::uninit();
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), size_of::<T>())
    };
    process.address_space().read(pointer as usize, bytes)?;
    Ok(unsafe { value.assume_init() })
}

fn validate_output<T>(process: &Process, pointer: *mut T) -> Result<(), Error> {
    if pointer.is_null() {
        return Err(Error::Fault);
    }
    if process
        .address_space()
        .user_range_accessible(pointer as usize, size_of::<T>(), true)
    {
        Ok(())
    } else {
        Err(Error::Fault)
    }
}

fn write_user_value<T: Copy>(process: &Process, pointer: *mut T, value: &T) -> Result<(), Error> {
    let bytes = unsafe {
        core::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>())
    };
    process.address_space().write(pointer as usize, bytes)
}

fn normalize_path(path: &[u8], current_directory: &[u8]) -> Result<Vec<u8>, c_int> {
    if path.is_empty() {
        return Err(as_errno(Error::NoEnt));
    }
    let base = if path.first() == Some(&b'/') {
        b"/".as_slice()
    } else {
        current_directory
    };
    let mut normalized = Vec::new();
    normalized
        .try_reserve(base.len().saturating_add(path.len()).saturating_add(1))
        .map_err(|_| ENOMEM)?;
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
        if normalized.len() + usize::from(normalized.len() > 1) + segment.len() > 4096 {
            return Err(as_errno(Error::NameTooLong));
        }
        if normalized.len() > 1 {
            normalized.push(b'/');
        }
        normalized.extend_from_slice(segment);
    }
    Ok(normalized)
}

fn read_user_string(process: &Process, pointer: *const c_char) -> Result<Vec<u8>, Error> {
    if pointer.is_null() {
        return Err(Error::Fault);
    }
    let mut output = Vec::new();
    for offset in 0..4096 {
        let address = (pointer as usize).checked_add(offset).ok_or(Error::Fault)?;
        let mut byte = [0];
        process.address_space().read(address, &mut byte)?;
        if byte[0] == 0 {
            return Ok(output);
        }
        output.try_reserve(1).map_err(|_| Error::NoMem)?;
        output.push(byte[0]);
    }
    Err(Error::NameTooLong)
}

fn c_path(path: &[u8]) -> Result<Vec<u8>, c_int> {
    let mut result = Vec::new();
    result.try_reserve_exact(path.len() + 1).map_err(|_| ENOMEM)?;
    result.extend_from_slice(path);
    result.push(0);
    Ok(result)
}

fn directory_generation() -> Result<usize, c_int> {
    NEXT_DIRECTORY_GENERATION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |generation| {
            (generation < HANDLE_GENERATION_MASK).then_some(generation + 1)
        })
        .map_err(|_| as_errno(Error::NoSpc))
}

fn allocate_directory(
    owner_pid: i32,
    descriptor: c_int,
    entries: Vec<crate::kernel::runtime::runtime_libc::filesystem::DirectoryEntry>,
    cursor: usize,
) -> Result<usize, c_int> {
    let generation = directory_generation()?;
    let mut directories = DIRECTORIES.lock();
    let slot = if let Some(slot) = directories.iter().position(|directory| directory.descriptor.is_none()) {
        slot
    } else {
        if directories.len() >= HANDLE_SLOT_MASK {
            return Err(ENOMEM);
        }
        directories.try_reserve(1).map_err(|_| ENOMEM)?;
        directories.push(Box::new(DirectoryStream {
            generation: 0,
            owner_pid,
            descriptor: None,
            entries: Vec::new(),
            cursor: 0,
            current: Dirent {
                inode: 0,
                name: [0; 256],
            },
        }));
        directories.len() - 1
    };
    *directories[slot] = DirectoryStream {
        generation,
        owner_pid,
        descriptor: Some(descriptor),
        entries,
        cursor,
        current: Dirent {
            inode: 0,
            name: [0; 256],
        },
    };
    Ok(((generation & HANDLE_GENERATION_MASK) << HANDLE_SLOT_BITS) | (slot + 1))
}

fn directory_slot(handle: *mut c_void, owner_pid: i32, directories: &[Box<DirectoryStream>]) -> Option<usize> {
    let token = handle as usize;
    let encoded_slot = token & HANDLE_SLOT_MASK;
    let generation = token >> HANDLE_SLOT_BITS;
    if encoded_slot == 0 || generation == 0 {
        return None;
    }
    let slot = encoded_slot - 1;
    let directory = directories.get(slot)?;
    (directory.generation == generation
        && directory.owner_pid == owner_pid
        && directory.descriptor.is_some())
        .then_some(slot)
}

fn entry_type(mode: u32) -> u8 {
    match mode & 0o170000 {
        0o040000 => DTYPE_DIR,
        0o100000 => DTYPE_REG,
        0o120000 => DTYPE_LNK,
        _ => DTYPE_UNKNOWN,
    }
}

fn to_dirent(
    entry: &crate::kernel::runtime::runtime_libc::filesystem::DirectoryEntry,
) -> Result<Dirent, c_int> {
    if entry.name.len() > NAME_MAX {
        return Err(as_errno(Error::NameTooLong));
    }
    let mut name = [0; 256];
    for (destination, source) in name.iter_mut().zip(&entry.name) {
        *destination = *source as c_char;
    }
    Ok(Dirent {
        inode: entry.inode,
        name,
    })
}

fn pack_getdents(
    entries: &[crate::kernel::runtime::runtime_libc::filesystem::DirectoryEntry],
    output: &mut [u8],
) -> Result<(usize, usize), c_int> {
    let mut written = 0;
    let mut consumed = 0;
    for entry in entries {
        if entry.name.len() > NAME_MAX {
            return Err(as_errno(Error::NameTooLong));
        }
        let unaligned_length = size_of::<u64>() * 2 + 1 + entry.name.len() + 1;
        let record_length = unaligned_length.saturating_add(7) & !7;
        if record_length > output.len().saturating_sub(written) {
            break;
        }
        let record = PosixDent {
            inode: entry.inode,
            record_length: record_length as u64,
            entry_type: entry_type(entry.mode),
            name: {
                let mut name = [0; 256];
                for (destination, source) in name.iter_mut().zip(&entry.name) {
                    *destination = *source as c_char;
                }
                name
            },
        };
        let record_bytes = unsafe {
            core::slice::from_raw_parts(
                (&record as *const PosixDent).cast::<u8>(),
                size_of::<PosixDent>(),
            )
        };
        output[written..written + record_length].fill(0);
        let copy_length = unaligned_length.min(record_bytes.len());
        output[written..written + copy_length].copy_from_slice(&record_bytes[..copy_length]);
        written += record_length;
        consumed += 1;
    }
    Ok((written, consumed))
}

unsafe fn compare_names(left: *const Dirent, right: *const Dirent) -> c_int {
    for index in 0..256 {
        let left_byte = unsafe { read_volatile((*left).name.as_ptr().add(index).cast::<u8>()) };
        let right_byte = unsafe { read_volatile((*right).name.as_ptr().add(index).cast::<u8>()) };
        if left_byte != right_byte {
            return left_byte as c_int - right_byte as c_int;
        }
        if left_byte == 0 {
            return 0;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn alphasort(
    left: *const *const Dirent,
    right: *const *const Dirent,
) -> c_int {
    if left.is_null() || right.is_null() {
        return match (left.is_null(), right.is_null()) {
            (true, true) => 0,
            (true, false) => -1,
            (false, true) => 1,
            (false, false) => 0,
        };
    }
    let left_entry = unsafe { read_volatile(left) };
    let right_entry = unsafe { read_volatile(right) };
    if left_entry.is_null() || right_entry.is_null() {
        return match (left_entry.is_null(), right_entry.is_null()) {
            (true, true) => 0,
            (true, false) => -1,
            (false, true) => 1,
            (false, false) => 0,
        };
    }
    unsafe { compare_names(left_entry, right_entry) }
}

#[no_mangle]
pub unsafe extern "C" fn closedir(directory: *mut c_void) -> c_int {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let mut directories = DIRECTORIES.lock();
    let Some(slot) = directory_slot(directory, process.pid(), &directories) else {
        set_errno(EBADF);
        return -1;
    };
    let descriptor = directories[slot].descriptor.take().unwrap();
    directories[slot].entries.clear();
    directories[slot].cursor = 0;
    drop(directories);
    if unsafe { close(descriptor) } == 0 {
        0
    } else {
        -1
    }
}

#[no_mangle]
pub unsafe extern "C" fn dirfd(directory: *mut c_void) -> c_int {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let directories = DIRECTORIES.lock();
    let Some(slot) = directory_slot(directory, process.pid(), &directories) else {
        set_errno(EBADF);
        return -1;
    };
    directories[slot].descriptor.unwrap_or_else(|| {
        set_errno(EBADF);
        -1
    })
}

#[no_mangle]
pub unsafe extern "C" fn fdopendir(file_descriptor: c_int) -> *mut c_void {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return core::ptr::null_mut();
        }
    };
    let entries = match crate::kernel::runtime::runtime_libc::filesystem::read_directory(
        file_descriptor,
    ) {
        Ok(entries) => entries,
        Err(error) => {
            set_errno(error);
            return core::ptr::null_mut();
        }
    };
    let cursor = match filesystem::directory_position(file_descriptor) {
        Ok(cursor) => cursor,
        Err(error) => {
            set_errno(error);
            return core::ptr::null_mut();
        }
    };
    match allocate_directory(process.pid(), file_descriptor, entries, cursor) {
        Ok(handle) => handle as *mut c_void,
        Err(error) => {
            set_errno(error);
            core::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn opendir(path: *const c_char) -> *mut c_void {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return core::ptr::null_mut();
        }
    };
    let path = match read_user_string(&process, path) {
        Ok(path) => path,
        Err(error) => {
            set_errno(as_errno(error));
            return core::ptr::null_mut();
        }
    };
    let current_directory = match process.current_directory() {
        Ok(directory) => directory,
        Err(error) => {
            set_errno(as_errno(error));
            return core::ptr::null_mut();
        }
    };
    let path = match normalize_path(&path, &current_directory).and_then(|path| c_path(&path)) {
        Ok(path) => path,
        Err(error) => {
            set_errno(error);
            return core::ptr::null_mut();
        }
    };
    let file_descriptor = unsafe {
        kz_rust_open(path.as_ptr().cast(), O_RDONLY | O_DIRECTORY, 0)
    };
    if file_descriptor < 0 {
        return core::ptr::null_mut();
    }
    let entries = match crate::kernel::runtime::runtime_libc::filesystem::read_directory(
        file_descriptor,
    ) {
        Ok(entries) => entries,
        Err(error) => {
            set_errno(error);
            let _ = unsafe { close(file_descriptor) };
            return core::ptr::null_mut();
        }
    };
    match allocate_directory(process.pid(), file_descriptor, entries, 0) {
        Ok(handle) => handle as *mut c_void,
        Err(error) => {
            set_errno(error);
            let _ = unsafe { close(file_descriptor) };
            core::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn posix_getdents(
    file_descriptor: c_int,
    buffer: *mut c_void,
    buffer_size: usize,
    flags: c_int,
) -> isize {
    if flags & !GETDENTS_SUPPORTED_FLAGS != 0 || buffer_size == 0 || buffer_size > (1 << 20) {
        set_errno(EINVAL);
        return -1;
    }
    if buffer.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    if !process
        .address_space()
        .user_range_accessible(buffer as usize, buffer_size, true)
    {
        set_errno(EFAULT);
        return -1;
    }
    let initial_position = match filesystem::directory_position(file_descriptor) {
        Ok(position) => position,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let mut output = Vec::new();
    if output.try_reserve_exact(buffer_size).is_err() {
        set_errno(ENOMEM);
        return -1;
    }
    output.resize(buffer_size, 0);
    let entries = match filesystem::read_directory(file_descriptor) {
        Ok(entries) => entries,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let Some(remaining_entries) = entries.get(initial_position..) else {
        set_errno(EINVAL);
        return -1;
    };
    let (written, consumed) = match pack_getdents(remaining_entries, &mut output) {
        Ok(result) => result,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    if consumed == 0 && !remaining_entries.is_empty() {
        set_errno(EINVAL);
        return -1;
    }
    if let Err(error) = process
        .address_space()
        .write(buffer as usize, &output[..written])
    {
        set_errno(as_errno(error));
        return -1;
    }
    if let Err(error) = filesystem::set_directory_position(
        file_descriptor,
        initial_position.saturating_add(consumed),
    ) {
        set_errno(error);
        return -1;
    }
    written as isize
}

#[no_mangle]
pub unsafe extern "C" fn readdir(directory: *mut c_void) -> *mut Dirent {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return core::ptr::null_mut();
        }
    };
    let mut directories = DIRECTORIES.lock();
    let Some(slot) = directory_slot(directory, process.pid(), &directories) else {
        set_errno(EBADF);
        return core::ptr::null_mut();
    };
    let stream = &mut *directories[slot];
    let Some(entry) = stream.entries.get(stream.cursor) else {
        return core::ptr::null_mut();
    };
    let current = match to_dirent(entry) {
        Ok(entry) => entry,
        Err(error) => {
            set_errno(error);
            return core::ptr::null_mut();
        }
    };
    stream.cursor += 1;
    stream.current = current;
    if let Some(descriptor) = stream.descriptor {
        let _ = crate::kernel::runtime::runtime_libc::filesystem::set_directory_position(
            descriptor,
            stream.cursor,
        );
    }
    &mut stream.current
}

#[no_mangle]
pub unsafe extern "C" fn readdir_r(
    directory: *mut c_void,
    entry: *mut Dirent,
    result: *mut *mut Dirent,
) -> c_int {
    if entry.is_null() || result.is_null() {
        return EFAULT;
    }
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => return error,
    };
    if let Err(error) = validate_output(&process, entry)
        .and_then(|()| validate_output(&process, result))
    {
        return as_errno(error);
    }
    let next = {
        let mut directories = DIRECTORIES.lock();
        let Some(slot) = directory_slot(directory, process.pid(), &directories) else {
            return EBADF;
        };
        let stream = &mut *directories[slot];
        let Some(directory_entry) = stream.entries.get(stream.cursor) else {
            if let Err(error) = write_user_value(&process, result, &mut core::ptr::null_mut()) {
                return as_errno(error);
            }
            return 0;
        };
        let directory_entry = match to_dirent(directory_entry) {
            Ok(entry) => entry,
            Err(error) => return error,
        };
        stream.cursor += 1;
        if let Some(descriptor) = stream.descriptor {
            let _ = crate::kernel::runtime::runtime_libc::filesystem::set_directory_position(
                descriptor,
                stream.cursor,
            );
        }
        directory_entry
    };
    if let Err(error) = write_user_value(&process, entry, &next) {
        return as_errno(error);
    }
    if let Err(error) = write_user_value(&process, result, &entry) {
        return as_errno(error);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn rewinddir(directory: *mut c_void) {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return;
        }
    };
    let mut directories = DIRECTORIES.lock();
    let Some(slot) = directory_slot(directory, process.pid(), &directories) else {
        set_errno(EBADF);
        return;
    };
    let stream = &mut *directories[slot];
    stream.cursor = 0;
    if let Some(descriptor) = stream.descriptor {
        if let Err(error) = crate::kernel::runtime::runtime_libc::filesystem::set_directory_position(
            descriptor, 0,
        ) {
            set_errno(error);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn scandir(
    path: *const c_char,
    entries: *mut *mut *mut Dirent,
    select: Option<unsafe extern "C" fn(*const Dirent) -> c_int>,
    compare: Option<
        unsafe extern "C" fn(*const *const Dirent, *const *const Dirent) -> c_int,
    >,
) -> c_int {
    if entries.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    if let Err(error) = validate_output(&process, entries) {
        set_errno(as_errno(error));
        return -1;
    }
    let directory = unsafe { opendir(path) };
    if directory.is_null() {
        return -1;
    }
    let mut selected = Vec::new();
    loop {
        let current = unsafe { readdir(directory) };
        if current.is_null() {
            break;
        }
        let entry = unsafe { *current };
        if select.is_some_and(|select| unsafe { select(&entry) == 0 }) {
            continue;
        }
        if selected.try_reserve(1).is_err() {
            let _ = unsafe { closedir(directory) };
            set_errno(ENOMEM);
            return -1;
        }
        selected.push(entry);
    }
    if unsafe { closedir(directory) } != 0 {
        return -1;
    }
    if let Some(compare) = compare {
        selected.sort_unstable_by(|left, right| {
            let left_pointer = left as *const Dirent;
            let right_pointer = right as *const Dirent;
            let order = unsafe { compare(&left_pointer, &right_pointer) };
            order.cmp(&0)
        });
    }

    let pointer_bytes = match selected
        .len()
        .checked_mul(size_of::<*mut Dirent>())
    {
        Some(size) => size.max(size_of::<*mut Dirent>()),
        None => {
            set_errno(ENOMEM);
            return -1;
        }
    };
    let pointer_array = unsafe { malloc(pointer_bytes) }.cast::<*mut Dirent>();
    if pointer_array.is_null() {
        set_errno(ENOMEM);
        return -1;
    }
    let mut allocated: Vec<*mut Dirent> = Vec::new();
    if allocated.try_reserve_exact(selected.len()).is_err() {
        unsafe { free(pointer_array.cast()) };
        set_errno(ENOMEM);
        return -1;
    }
    for (index, entry) in selected.iter().enumerate() {
        let output_entry = unsafe { malloc(size_of::<Dirent>()) }.cast::<Dirent>();
        if output_entry.is_null() {
            for pointer in allocated {
                unsafe { free(pointer.cast()) };
            }
            unsafe { free(pointer_array.cast()) };
            set_errno(ENOMEM);
            return -1;
        }
        if let Err(error) = write_user_value(&process, output_entry, entry) {
            unsafe { free(output_entry.cast()) };
            for pointer in allocated {
                unsafe { free(pointer.cast()) };
            }
            unsafe { free(pointer_array.cast()) };
            set_errno(as_errno(error));
            return -1;
        }
        allocated.push(output_entry);
        let entry_pointer = output_entry;
        let array_slot = unsafe { pointer_array.add(index) };
        if let Err(error) = write_user_value(&process, array_slot, &entry_pointer) {
            for pointer in allocated {
                unsafe { free(pointer.cast()) };
            }
            unsafe { free(pointer_array.cast()) };
            set_errno(as_errno(error));
            return -1;
        }
    }
    if let Err(error) = write_user_value(&process, entries, &pointer_array) {
        for pointer in allocated {
            unsafe { free(pointer.cast()) };
        }
        unsafe { free(pointer_array.cast()) };
        set_errno(as_errno(error));
        return -1;
    }
    selected.len() as c_int
}

#[no_mangle]
pub unsafe extern "C" fn seekdir(directory: *mut c_void, position: c_long) {
    if position < 0 {
        set_errno(EINVAL);
        return;
    }
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return;
        }
    };
    let mut directories = DIRECTORIES.lock();
    let Some(slot) = directory_slot(directory, process.pid(), &directories) else {
        set_errno(EBADF);
        return;
    };
    let Ok(position) = usize::try_from(position) else {
        set_errno(EINVAL);
        return;
    };
    if position > directories[slot].entries.len() {
        set_errno(EINVAL);
        return;
    }
    directories[slot].cursor = position;
    if let Some(descriptor) = directories[slot].descriptor {
        if let Err(error) = filesystem::set_directory_position(descriptor, position) {
            set_errno(error);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn telldir(directory: *mut c_void) -> c_long {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let directories = DIRECTORIES.lock();
    let Some(slot) = directory_slot(directory, process.pid(), &directories) else {
        set_errno(EBADF);
        return -1;
    };
    match c_long::try_from(directories[slot].cursor) {
        Ok(position) => position,
        Err(_) => {
            set_errno(EINVAL);
            -1
        }
    }
}