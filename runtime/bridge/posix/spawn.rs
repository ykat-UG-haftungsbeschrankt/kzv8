/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_void};
use core::mem::{size_of, MaybeUninit};

use crate::kernel::dl::AddressSpace as _;
use crate::kernel::error::Error;
use crate::kernel::process::{Process, SchedulingParameters};
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::signal::SignalSet;

use super::errno::as_errno;
use super::exec::read_guest_string;
use super::sched::{self, SchedParam};

const POSIX_SPAWN_RESETIDS: i16 = 0x01;
const POSIX_SPAWN_SETPGROUP: i16 = 0x02;
const POSIX_SPAWN_SETSIGDEF: i16 = 0x04;
const POSIX_SPAWN_SETSIGMASK: i16 = 0x08;
const POSIX_SPAWN_SETSCHEDPARAM: i16 = 0x10;
const POSIX_SPAWN_SETSCHEDULER: i16 = 0x20;
const POSIX_SPAWN_SETSID: i16 = 0x40;
const POSIX_SPAWN_SUPPORTED_FLAGS: i16 = POSIX_SPAWN_RESETIDS
    | POSIX_SPAWN_SETPGROUP
    | POSIX_SPAWN_SETSIGDEF
    | POSIX_SPAWN_SETSIGMASK
    | POSIX_SPAWN_SETSCHEDPARAM
    | POSIX_SPAWN_SETSCHEDULER
    | POSIX_SPAWN_SETSID;

#[derive(Clone, Copy)]
pub(crate) struct SpawnAttributes {
    pub(crate) flags: i16,
    pub(crate) process_group: i32,
    pub(crate) scheduling: SchedulingParameters,
    pub(crate) signal_default: SignalSet,
    pub(crate) signal_mask: SignalSet,
}

struct SpawnAttributesEntry {
    address: usize,
    attributes: SpawnAttributes,
}

pub(super) enum SpawnFileAction {
    Close(i32),
    Dup2 { source: i32, destination: i32 },
    Open {
        file_descriptor: i32,
        path: Vec<u8>,
        flags: i32,
        mode: u32,
    },
    Chdir(Vec<u8>),
    Fchdir(i32),
}

struct SpawnFileActionsEntry {
    address: usize,
    actions: Vec<SpawnFileAction>,
}

static SPAWN_ATTRIBUTES: IrqSpinLock<Vec<SpawnAttributesEntry>> = IrqSpinLock::new(Vec::new());
static SPAWN_FILE_ACTIONS: IrqSpinLock<Vec<SpawnFileActionsEntry>> = IrqSpinLock::new(Vec::new());

fn current_process() -> Result<alloc::sync::Arc<Process>, c_int> {
    ThreadManager::current_process().map_err(as_errno)
}

pub(crate) fn spawn_attributes(address: *const c_void) -> Result<Option<SpawnAttributes>, c_int> {
    if address.is_null() {
        return Ok(None);
    }
    SPAWN_ATTRIBUTES
        .lock()
        .iter()
        .find(|entry| entry.address == address as usize)
        .map(|entry| Some(entry.attributes))
        .ok_or(as_errno(Error::Inval))
}

pub(super) fn file_actions_snapshot(
    address: *const c_void,
) -> Result<Vec<SpawnFileAction>, c_int> {
    if address.is_null() {
        return Ok(Vec::new());
    }
    let entries = SPAWN_FILE_ACTIONS.lock();
    let actions = entries
        .iter()
        .find(|entry| entry.address == address as usize)
        .map(|entry| &entry.actions)
        .ok_or(as_errno(Error::Inval))?;
    let mut snapshot = Vec::new();
    snapshot.try_reserve_exact(actions.len()).map_err(|_| as_errno(Error::NoMem))?;
    for action in actions {
        snapshot.push(clone_file_action(action)?);
    }
    Ok(snapshot)
}

fn clone_file_action(action: &SpawnFileAction) -> Result<SpawnFileAction, c_int> {
    match action {
        SpawnFileAction::Close(file_descriptor) => Ok(SpawnFileAction::Close(*file_descriptor)),
        SpawnFileAction::Dup2 { source, destination } => Ok(SpawnFileAction::Dup2 {
            source: *source,
            destination: *destination,
        }),
        SpawnFileAction::Open {
            file_descriptor,
            path,
            flags,
            mode,
        } => {
            let mut path_copy = Vec::new();
            path_copy
                .try_reserve_exact(path.len())
                .map_err(|_| as_errno(Error::NoMem))?;
            path_copy.extend_from_slice(path);
            Ok(SpawnFileAction::Open {
                file_descriptor: *file_descriptor,
                path: path_copy,
                flags: *flags,
                mode: *mode,
            })
        }
        SpawnFileAction::Chdir(path) => {
            let mut path_copy = Vec::new();
            path_copy
                .try_reserve_exact(path.len())
                .map_err(|_| as_errno(Error::NoMem))?;
            path_copy.extend_from_slice(path);
            Ok(SpawnFileAction::Chdir(path_copy))
        }
        SpawnFileAction::Fchdir(file_descriptor) => {
            Ok(SpawnFileAction::Fchdir(*file_descriptor))
        }
    }
}

fn read_signal_set(process: &Process, address: *const c_void) -> Result<SignalSet, c_int> {
    if address.is_null() {
        return Err(as_errno(Error::Fault));
    }
    let mut bytes = [0; size_of::<u64>()];
    process
        .address_space()
        .read(address as usize, &mut bytes)
        .map_err(as_errno)?;
    Ok(SignalSet::from_bits(u64::from_ne_bytes(bytes)))
}

fn write_signal_set(
    process: &Process,
    address: *mut c_void,
    signal_set: SignalSet,
) -> Result<(), c_int> {
    if address.is_null() {
        return Err(as_errno(Error::Fault));
    }
    let mut bytes = [0; 128];
    bytes[..size_of::<u64>()].copy_from_slice(&signal_set.bits().to_ne_bytes());
    process
        .address_space()
        .write(address as usize, &bytes)
        .map_err(as_errno)
}

fn update_attributes(
    address: *mut c_void,
    update: impl FnOnce(&mut SpawnAttributes) -> Result<(), Error>,
) -> c_int {
    if address.is_null() {
        return as_errno(Error::Inval);
    }
    let mut entries = SPAWN_ATTRIBUTES.lock();
    let Some(entry) = entries.iter_mut().find(|entry| entry.address == address as usize) else {
        return as_errno(Error::Inval);
    };
    match update(&mut entry.attributes) {
        Ok(()) => 0,
        Err(error) => as_errno(error),
    }
}

fn read_spawn_sched_param(parameters: *const c_void) -> Result<SchedParam, c_int> {
    if parameters.is_null() {
        return Err(as_errno(Error::Fault));
    }
    let process = current_process()?;
    let mut value = MaybeUninit::<SchedParam>::uninit();
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), size_of::<SchedParam>())
    };
    process
        .address_space()
        .read(parameters as usize, bytes)
        .map_err(as_errno)?;
    Ok(unsafe { value.assume_init() })
}

fn write_user_value<T: Copy>(output: *mut T, value: &T) -> Result<(), c_int> {
    if output.is_null() {
        return Err(as_errno(Error::Fault));
    }
    let process = current_process()?;
    let bytes = unsafe {
        core::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>())
    };
    process
        .address_space()
        .write(output as usize, bytes)
        .map_err(as_errno)
}

fn get_spawn_attributes(address: *const c_void) -> Result<SpawnAttributes, c_int> {
    if address.is_null() {
        return Err(as_errno(Error::Inval));
    }
    SPAWN_ATTRIBUTES
        .lock()
        .iter()
        .find(|entry| entry.address == address as usize)
        .map(|entry| entry.attributes)
        .ok_or(as_errno(Error::Inval))
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_getflags(
    attributes: *const c_void,
    flags: *mut i16,
) -> c_int {
    match get_spawn_attributes(attributes) {
        Ok(attributes) => write_user_value(flags, &attributes.flags).map_or_else(|error| error, |_| 0),
        Err(error) => error,
    }
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_setflags(
    attributes: *mut c_void,
    flags: i16,
) -> c_int {
    if flags & !POSIX_SPAWN_SUPPORTED_FLAGS != 0 {
        return as_errno(Error::Inval);
    }
    update_attributes(attributes, |attributes| {
        attributes.flags = flags;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_getpgroup(
    attributes: *const c_void,
    process_group: *mut i32,
) -> c_int {
    match get_spawn_attributes(attributes) {
        Ok(attributes) => write_user_value(process_group, &attributes.process_group)
            .map_or_else(|error| error, |_| 0),
        Err(error) => error,
    }
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_setpgroup(
    attributes: *mut c_void,
    process_group: i32,
) -> c_int {
    if process_group < 0 {
        return as_errno(Error::Inval);
    }
    update_attributes(attributes, |attributes| {
        attributes.process_group = process_group;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_getschedpolicy(
    attributes: *const c_void,
    policy: *mut c_int,
) -> c_int {
    match get_spawn_attributes(attributes) {
        Ok(attributes) => write_user_value(policy, &attributes.scheduling.policy)
            .map_or_else(|error| error, |_| 0),
        Err(error) => error,
    }
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_setschedpolicy(
    attributes: *mut c_void,
    policy: c_int,
) -> c_int {
    if !matches!(policy, 0..=3) {
        return as_errno(Error::Inval);
    }
    update_attributes(attributes, |attributes| {
        attributes.scheduling.policy = policy;
        if policy != 3 {
            attributes.scheduling.sporadic = None;
        }
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_getschedparam(
    attributes: *const c_void,
    parameters: *mut c_void,
) -> c_int {
    let attributes = match get_spawn_attributes(attributes) {
        Ok(attributes) => attributes,
        Err(error) => return error,
    };
    let parameter = sched::sched_param_for_configuration(attributes.scheduling);
    if parameters.is_null() {
        return as_errno(Error::Fault);
    }
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => return error,
    };
    let bytes = unsafe {
        core::slice::from_raw_parts((&parameter as *const SchedParam).cast::<u8>(), size_of::<SchedParam>())
    };
    process.address_space().write(parameters as usize, bytes).map_or_else(as_errno, |_| 0)
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_setschedparam(
    attributes: *mut c_void,
    parameters: *const c_void,
) -> c_int {
    let parameter = match read_spawn_sched_param(parameters) {
        Ok(parameter) => parameter,
        Err(error) => return error,
    };
    let policy = match get_spawn_attributes(attributes) {
        Ok(attributes) => attributes.scheduling.policy,
        Err(error) => return error,
    };
    let scheduling = match sched::scheduling_configuration_from_param(policy, parameter) {
        Ok(scheduling) => scheduling,
        Err(error) => return as_errno(error),
    };
    update_attributes(attributes, |attributes| {
        attributes.scheduling = scheduling;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_getsigmask(
    attributes: *const c_void,
    signal_set: *mut c_void,
) -> c_int {
    let attributes = match get_spawn_attributes(attributes) {
        Ok(attributes) => attributes,
        Err(error) => return error,
    };
    match current_process() {
        Ok(process) => write_signal_set(&process, signal_set, attributes.signal_mask)
            .map_or_else(|error| error, |_| 0),
        Err(error) => error,
    }
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_getsigdefault(
    attributes: *const c_void,
    signal_set: *mut c_void,
) -> c_int {
    match get_spawn_attributes(attributes) {
        Ok(attributes) => match current_process() {
            Ok(process) => write_signal_set(&process, signal_set, attributes.signal_default)
                .map_or_else(|error| error, |_| 0),
            Err(error) => error,
        },
        Err(error) => error,
    }
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_setsigmask(
    attributes: *mut c_void,
    signal_set: *const c_void,
) -> c_int {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => return error,
    };
    let signal_set = match read_signal_set(&process, signal_set) {
        Ok(signal_set) => signal_set,
        Err(error) => return error,
    };
    update_attributes(attributes, |attributes| {
        attributes.signal_mask = signal_set;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_setsigdefault(
    attributes: *mut c_void,
    signal_set: *const c_void,
) -> c_int {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => return error,
    };
    let signal_set = match read_signal_set(&process, signal_set) {
        Ok(signal_set) => signal_set,
        Err(error) => return error,
    };
    update_attributes(attributes, |attributes| {
        attributes.signal_default = signal_set;
        Ok(())
    })
}

fn append_file_action(actions: *mut c_void, action: SpawnFileAction) -> c_int {
    if actions.is_null() {
        return as_errno(Error::Inval);
    }
    let mut entries = SPAWN_FILE_ACTIONS.lock();
    let Some(entry) = entries
        .iter_mut()
        .find(|entry| entry.address == actions as usize)
    else {
        return as_errno(Error::Inval);
    };
    if entry.actions.try_reserve(1).is_err() {
        return as_errno(Error::NoMem);
    }
    entry.actions.push(action);
    0
}

fn read_file_action_path(path: *const c_char) -> Result<Vec<u8>, c_int> {
    let process = current_process()?;
    read_guest_string(&process, path as usize)
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawn(
    process_id: *mut i32,
    path: *const c_char,
    file_actions: *const c_void,
    attributes: *const c_void,
    arguments: *const *mut c_char,
    environment: *const *mut c_char,
) -> c_int {
    unsafe {
        super::exec::posix_spawn(
            process_id,
            path,
            file_actions,
            attributes,
            arguments,
            environment,
            false,
        )
    }
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnp(
    process_id: *mut i32,
    file: *const c_char,
    file_actions: *const c_void,
    attributes: *const c_void,
    arguments: *const *mut c_char,
    environment: *const *mut c_char,
) -> c_int {
    unsafe {
        super::exec::posix_spawn(
            process_id,
            file,
            file_actions,
            attributes,
            arguments,
            environment,
            true,
        )
    }
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawn_file_actions_addchdir(
    actions: *mut c_void,
    path: *const c_char,
) -> c_int {
    let path = match read_file_action_path(path) {
        Ok(path) => path,
        Err(error) => return error,
    };
    append_file_action(actions, SpawnFileAction::Chdir(path))
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawn_file_actions_addclose(
    actions: *mut c_void,
    file_descriptor: c_int,
) -> c_int {
    if file_descriptor < 0 {
        return as_errno(Error::Inval);
    }
    append_file_action(actions, SpawnFileAction::Close(file_descriptor))
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawn_file_actions_adddup2(
    actions: *mut c_void,
    source: c_int,
    destination: c_int,
) -> c_int {
    if source < 0 || destination < 0 {
        return as_errno(Error::Inval);
    }
    append_file_action(
        actions,
        SpawnFileAction::Dup2 { source, destination },
    )
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawn_file_actions_addfchdir(
    actions: *mut c_void,
    file_descriptor: c_int,
) -> c_int {
    if file_descriptor < 0 {
        return as_errno(Error::Inval);
    }
    append_file_action(actions, SpawnFileAction::Fchdir(file_descriptor))
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawn_file_actions_addopen(
    actions: *mut c_void,
    file_descriptor: c_int,
    path: *const c_char,
    flags: c_int,
    mode: u32,
) -> c_int {
    if file_descriptor < 0 {
        return as_errno(Error::Inval);
    }
    let path = match read_file_action_path(path) {
        Ok(path) => path,
        Err(error) => return error,
    };
    append_file_action(
        actions,
        SpawnFileAction::Open {
            file_descriptor,
            path,
            flags,
            mode,
        },
    )
}
#[no_mangle]
pub unsafe extern "C" fn posix_spawn_file_actions_init(actions: *mut c_void) -> c_int {
    if actions.is_null() {
        return as_errno(Error::Inval);
    }
    let mut entries = SPAWN_FILE_ACTIONS.lock();
    if entries.iter().any(|entry| entry.address == actions as usize) {
        return as_errno(Error::Inval);
    }
    if entries.try_reserve(1).is_err() {
        return as_errno(Error::NoMem);
    }
    entries.push(SpawnFileActionsEntry {
        address: actions as usize,
        actions: Vec::new(),
    });
    0
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawn_file_actions_destroy(actions: *mut c_void) -> c_int {
    if actions.is_null() {
        return as_errno(Error::Inval);
    }
    let mut entries = SPAWN_FILE_ACTIONS.lock();
    let Some(index) = entries.iter().position(|entry| entry.address == actions as usize) else {
        return as_errno(Error::Inval);
    };
    entries.swap_remove(index);
    0
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_init(attributes: *mut c_void) -> c_int {
    if attributes.is_null() {
        return as_errno(Error::Inval);
    }
    let mut entries = SPAWN_ATTRIBUTES.lock();
    if entries.iter().any(|entry| entry.address == attributes as usize) {
        return as_errno(Error::Inval);
    }
    if entries.try_reserve(1).is_err() {
        return as_errno(Error::NoMem);
    }
    entries.push(SpawnAttributesEntry {
        address: attributes as usize,
        attributes: SpawnAttributes {
            flags: 0,
            process_group: 0,
            scheduling: SchedulingParameters::OTHER,
            signal_default: SignalSet::EMPTY,
            signal_mask: SignalSet::EMPTY,
        },
    });
    0
}

#[no_mangle]
pub unsafe extern "C" fn posix_spawnattr_destroy(attributes: *mut c_void) -> c_int {
    if attributes.is_null() {
        return as_errno(Error::Inval);
    }
    let mut entries = SPAWN_ATTRIBUTES.lock();
    let Some(index) = entries.iter().position(|entry| entry.address == attributes as usize) else {
        return as_errno(Error::Inval);
    };
    entries.swap_remove(index);
    0
}