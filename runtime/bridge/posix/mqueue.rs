/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_long, c_void};
use core::mem::{size_of, MaybeUninit};

use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::scheduler::thread::{ThreadManager, ThreadStack};
use crate::kernel::signal::{SignalInfo, SI_QUEUE, MAX_SIGNAL_NUMBER};

use super::errno::{as_errno, EAGAIN, EBADF, EFAULT, EINVAL, ENAMETOOLONG, ENOMEM};

const O_ACCMODE: c_int = 0o3;
const O_RDONLY: c_int = 0;
const O_WRONLY: c_int = 1;
const O_RDWR: c_int = 2;
const O_CREAT: c_int = 0o100;
const O_EXCL: c_int = 0o200;
const O_NONBLOCK: c_int = 0o4000;
const O_CLOEXEC: c_int = 0o2000000;
const MAX_QUEUES: usize = 32;
const MAX_OPEN_PER_PROCESS: usize = 8;
const MAX_DESCRIPTORS: usize = 256;
const MAX_MESSAGES: usize = 64;
const MAX_MESSAGE_SIZE: usize = 4096;
const MAX_MESSAGE_PRIORITY: u32 = 32767;
const HANDLE_SLOT_BITS: u32 = 9;
const HANDLE_SLOT_MASK: u32 = (1 << HANDLE_SLOT_BITS) - 1;
const HANDLE_GENERATION_MAX: u32 = (i32::MAX as u32) >> HANDLE_SLOT_BITS;
const CLOCK_REALTIME: c_int = 0;
const CLOCK_MONOTONIC: c_int = 1;
const CLOCK_MONOTONIC_RAW: c_int = 4;
const ETIMEDOUT: c_int = 110;
const SIGEV_SIGNAL: c_int = 0;
const SIGEV_NONE: c_int = 1;
const SIGEV_THREAD: c_int = 2;

#[repr(C)]
#[derive(Clone, Copy)]
union Sigval {
    integer: c_int,
    pointer: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Sigevent {
    notify: c_int,
    signal: c_int,
    value: Sigval,
    function: Option<unsafe extern "C" fn(Sigval)>,
    attributes: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MqAttr {
    flags: c_long,
    max_messages: c_long,
    message_size: c_long,
    current_messages: c_long,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Timespec {
    seconds: i64,
    nanoseconds: i64,
}

#[derive(Clone)]
struct Message {
    bytes: Vec<u8>,
    priority: u32,
}

#[derive(Clone)]
enum MqNotification {
    None {
        process: Arc<Process>,
    },
    Signal {
        process: Arc<Process>,
        signal: u8,
        value: usize,
    },
    Thread {
        process: Arc<Process>,
        function: unsafe extern "C" fn(Sigval),
        value: usize,
    },
}

struct MqThreadContext {
    function: unsafe extern "C" fn(Sigval),
    value: usize,
}

unsafe extern "C" fn mqueue_notify_thread(argument: *mut c_void) -> *mut c_void {
    let context = unsafe { Box::from_raw(argument.cast::<MqThreadContext>()) };
    unsafe {
        (context.function)(Sigval {
            pointer: context.value as *mut c_void,
        })
    };
    core::ptr::null_mut()
}

fn dispatch_notification(notification: MqNotification) {
    match notification {
        MqNotification::None { process } => {
            let _ = process.pid();
        }
        MqNotification::Signal {
            process,
            signal,
            value,
        } => {
            let _ = process.queue_signal(SignalInfo::new(
                signal,
                SI_QUEUE,
                0,
                0,
                value,
                0,
            ));
        }
        MqNotification::Thread {
            process,
            function,
            value,
        } => {
            let argument = Box::into_raw(Box::new(MqThreadContext { function, value }))
                .cast::<c_void>();
            if ThreadManager::create_thread_in_process(
                process,
                ThreadStack::DEFAULT_SIZE,
                mqueue_notify_thread,
                argument,
                true,
            )
            .is_err()
            {
                unsafe { drop(Box::from_raw(argument.cast::<MqThreadContext>())) };
            }
        }
    }
}

struct MessageQueue {
    name: Vec<u8>,
    messages: Vec<Message>,
    max_messages: usize,
    message_size: usize,
    waiting_receivers: usize,
    references: usize,
    unlinked: bool,
    notification: Option<MqNotification>,
}

#[derive(Clone, Copy)]
struct MqDescriptor {
    generation: u32,
    active: bool,
    owner_pid: i32,
    queue_index: usize,
    readable: bool,
    writable: bool,
    nonblocking: bool,
    close_on_exec: bool,
}

struct MqState {
    queues: Vec<Option<MessageQueue>>,
    descriptors: [MqDescriptor; MAX_DESCRIPTORS],
}

impl MqState {
    const fn new() -> Self {
        Self {
            queues: Vec::new(),
            descriptors: [MqDescriptor {
                generation: 0,
                active: false,
                owner_pid: 0,
                queue_index: 0,
                readable: false,
                writable: false,
                nonblocking: false,
                close_on_exec: false,
            }; MAX_DESCRIPTORS],
        }
    }
}

static MQUEUE_STATE: IrqSpinLock<MqState> = IrqSpinLock::new(MqState::new());

unsafe extern "C" {
    fn clock_gettime(clock_id: c_int, output: *mut Timespec) -> c_int;
    fn pthread_yield() -> c_int;
}

fn read_message_bytes(process: &Process, pointer: *const c_char, size: usize) -> Result<Vec<u8>, c_int> {
    if pointer.is_null() && size != 0 {
        return Err(EFAULT);
    }
    let mut message = Vec::new();
    message.try_reserve_exact(size).map_err(|_| ENOMEM)?;
    message.resize(size, 0);
    if size != 0 {
        process
            .address_space()
            .read(pointer as usize, &mut message)
            .map_err(as_errno)?;
    }
    Ok(message)
}

fn absolute_timeout(clock_id: c_int, timeout: *const c_void) -> Result<Option<i128>, c_int> {
    if timeout.is_null() {
        return Err(EFAULT);
    }
    if !matches!(clock_id, CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW) {
        return Err(EINVAL);
    }
    let process = current_process()?;
    let timeout = read_user_value::<Timespec>(&process, timeout.cast())?;
    if timeout.seconds < 0 || !(0..1_000_000_000).contains(&timeout.nanoseconds) {
        return Err(EINVAL);
    }
    Ok(Some(i128::from(timeout.seconds) * 1_000_000_000 + i128::from(timeout.nanoseconds)))
}

fn deadline_expired(clock_id: c_int, deadline_ns: Option<i128>) -> Result<bool, c_int> {
    let Some(deadline_ns) = deadline_ns else {
        return Ok(false);
    };
    let mut now = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    if unsafe { clock_gettime(clock_id, &mut now) } != 0 {
        return Err(unsafe { crate::kernel::scheduler::thread::__errno_location().read() });
    }
    Ok(i128::from(now.seconds) * 1_000_000_000 + i128::from(now.nanoseconds) >= deadline_ns)
}

fn send_message(
    descriptor: c_int,
    message: Vec<u8>,
    priority: u32,
    clock_id: c_int,
    deadline_ns: Option<i128>,
) -> Result<(), c_int> {
    if priority > MAX_MESSAGE_PRIORITY {
        return Err(EINVAL);
    }
    let process = current_process()?;
    loop {
        ThreadManager::cancel_current_if_requested();
        let (result, notification, nonblocking) = {
            let mut state = MQUEUE_STATE.lock();
            let (descriptor_slot, queue_index) = descriptor_parts(descriptor, process.pid(), &state)?;
            let descriptor_info = state.descriptors[descriptor_slot];
            let nonblocking = descriptor_info.nonblocking;
            if !descriptor_info.writable {
                return Err(EBADF);
            }
            let queue = state.queues.get_mut(queue_index).and_then(Option::as_mut).ok_or(EBADF)?;
            if message.len() > queue.message_size {
                return Err(as_errno(Error::MsgSize));
            }
            if queue.messages.len() < queue.max_messages {
                let was_empty = queue.messages.is_empty();
                let mut copy = Vec::new();
                copy.try_reserve_exact(message.len()).map_err(|_| ENOMEM)?;
                copy.extend_from_slice(&message);
                let insert_at = queue
                    .messages
                    .iter()
                    .position(|existing| existing.priority < priority)
                    .unwrap_or(queue.messages.len());
                queue.messages.try_reserve(1).map_err(|_| ENOMEM)?;
                queue.messages.insert(insert_at, Message { bytes: copy, priority });
                let notification = (was_empty && queue.waiting_receivers == 0)
                    .then(|| queue.notification.take())
                    .flatten();
                (Ok(()), notification, nonblocking)
            } else if descriptor_info.nonblocking {
                (Err(EAGAIN), None, nonblocking)
            } else {
                (Err(EAGAIN), None, nonblocking)
            }
        };
        match result {
            Ok(()) => {
                if let Some(notification) = notification {
                    dispatch_notification(notification);
                }
                process.record_message_sent();
                return Ok(());
            }
            Err(EAGAIN) => {
                if nonblocking {
                    return Err(EAGAIN);
                }
                if deadline_expired(clock_id, deadline_ns)? {
                    return Err(ETIMEDOUT);
                }
                if crate::kernel::runtime::runtime_posix::signal::dispatch_pending_signals()
                    .is_some_and(|restart| !restart)
                {
                    return Err(as_errno(Error::Intr));
                }
                unsafe { pthread_yield() };
            }
            Err(error) => return Err(error),
        }
    }
}

fn unregister_receiver(descriptor: c_int, process_id: i32) {
    let mut state = MQUEUE_STATE.lock();
    let Ok((_, queue_index)) = descriptor_parts(descriptor, process_id, &state) else {
        return;
    };
    if let Some(queue) = state.queues.get_mut(queue_index).and_then(Option::as_mut) {
        queue.waiting_receivers = queue.waiting_receivers.saturating_sub(1);
    }
}

fn receive_message(
    descriptor: c_int,
    message: *mut c_char,
    message_size: usize,
    priority: *mut u32,
    clock_id: c_int,
    deadline_ns: Option<i128>,
) -> Result<isize, c_int> {
    if message.is_null() {
        return Err(EFAULT);
    }
    let process = current_process()?;
    if !process
        .address_space()
        .user_range_accessible(message as usize, message_size, true)
    {
        return Err(EFAULT);
    }
    if !priority.is_null()
        && !process
            .address_space()
            .user_range_accessible(priority as usize, size_of::<u32>(), true)
    {
        return Err(EFAULT);
    }
    let mut registered_waiter = false;
    loop {
        if ThreadManager::cancellation_requested_current() && registered_waiter {
            unregister_receiver(descriptor, process.pid());
            registered_waiter = false;
        }
        ThreadManager::cancel_current_if_requested();
        let (result, nonblocking) = {
            let mut state = MQUEUE_STATE.lock();
            let (descriptor_slot, queue_index) = descriptor_parts(descriptor, process.pid(), &state)?;
            let descriptor_info = state.descriptors[descriptor_slot];
            let nonblocking = descriptor_info.nonblocking;
            if !descriptor_info.readable {
                return Err(EBADF);
            }
            let queue = state.queues.get_mut(queue_index).and_then(Option::as_mut).ok_or(EBADF)?;
            if message_size < queue.message_size {
                return Err(as_errno(Error::MsgSize));
            }
            if !queue.messages.is_empty() {
                if registered_waiter {
                    queue.waiting_receivers = queue.waiting_receivers.saturating_sub(1);
                    registered_waiter = false;
                }
                (Ok(queue.messages.remove(0)), nonblocking)
            } else if descriptor_info.nonblocking {
                (Err(EAGAIN), nonblocking)
            } else {
                if !registered_waiter {
                    queue.waiting_receivers = queue.waiting_receivers.saturating_add(1);
                    registered_waiter = true;
                }
                (Err(EAGAIN), nonblocking)
            }
        };
        match result {
            Ok(received) => {
                process
                    .address_space()
                    .write(message as usize, &received.bytes)
                    .map_err(as_errno)?;
                if !priority.is_null() {
                    write_user_value(&process, priority, &received.priority)?;
                }
                process.record_message_received();
                return isize::try_from(received.bytes.len()).map_err(|_| EINVAL);
            }
            Err(EAGAIN) => {
                if nonblocking {
                    return Err(EAGAIN);
                }
                match deadline_expired(clock_id, deadline_ns) {
                    Ok(true) => {
                        unregister_receiver(descriptor, process.pid());
                        return Err(ETIMEDOUT);
                    }
                    Err(error) => {
                        unregister_receiver(descriptor, process.pid());
                        return Err(error);
                    }
                    Ok(false) => {}
                }
                if crate::kernel::runtime::runtime_posix::signal::dispatch_pending_signals()
                    .is_some_and(|restart| !restart)
                {
                    unregister_receiver(descriptor, process.pid());
                    return Err(as_errno(Error::Intr));
                }
                unsafe { pthread_yield() };
            }
            Err(error) => return Err(error),
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_mq_open(
    name: *const c_char,
    flags: c_int,
    _mode: u32,
    attributes: *const MqAttr,
) -> c_int {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let name = match read_user_name(&process, name) {
        Ok(name) => name,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let attributes = if flags & O_CREAT != 0 && !attributes.is_null() {
        match read_user_value(&process, attributes) {
            Ok(attributes) => Some(attributes),
            Err(error) => {
                set_errno(error);
                return -1;
            }
        }
    } else {
        None
    };
    match open_queue(process.pid(), &name, flags, attributes) {
        Ok(descriptor) => descriptor,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

fn current_process() -> Result<alloc::sync::Arc<Process>, c_int> {
    ThreadManager::current_process().map_err(as_errno)
}

fn set_errno(error: c_int) {
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
}

fn read_user_value<T: Copy>(process: &Process, pointer: *const T) -> Result<T, c_int> {
    if pointer.is_null() {
        return Err(EFAULT);
    }
    let mut value = MaybeUninit::<T>::uninit();
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), size_of::<T>())
    };
    process
        .address_space()
        .read(pointer as usize, bytes)
        .map_err(as_errno)?;
    Ok(unsafe { value.assume_init() })
}

fn write_user_value<T: Copy>(process: &Process, pointer: *mut T, value: &T) -> Result<(), c_int> {
    if pointer.is_null() {
        return Err(EFAULT);
    }
    let bytes = unsafe { core::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) };
    process.address_space().write(pointer as usize, bytes).map_err(as_errno)
}

fn read_user_name(process: &Process, pointer: *const c_char) -> Result<Vec<u8>, c_int> {
    if pointer.is_null() {
        return Err(EFAULT);
    }
    let mut name = Vec::new();
    for offset in 0..256 {
        let address = (pointer as usize).checked_add(offset).ok_or(EFAULT)?;
        let mut byte = [0];
        process.address_space().read(address, &mut byte).map_err(as_errno)?;
        if byte[0] == 0 {
            if name.len() < 2 || name[0] != b'/' || name[1..].contains(&b'/') {
                return Err(EINVAL);
            }
            return Ok(name);
        }
        name.try_reserve(1).map_err(|_| ENOMEM)?;
        name.push(byte[0]);
    }
    Err(ENAMETOOLONG)
}

fn descriptor_parts(descriptor: c_int, pid: i32, state: &MqState) -> Result<(usize, usize), c_int> {
    if descriptor <= 0 {
        return Err(EBADF);
    }
    let token = descriptor as u32;
    let slot = (token & HANDLE_SLOT_MASK) as usize;
    let generation = token >> HANDLE_SLOT_BITS;
    if slot == 0 || generation == 0 {
        return Err(EBADF);
    }
    let slot = slot - 1;
    let entry = state.descriptors.get(slot).ok_or(EBADF)?;
    if !entry.active || entry.generation != generation || entry.owner_pid != pid {
        return Err(EBADF);
    }
    Ok((slot, entry.queue_index))
}

fn allocate_descriptor(
    state: &mut MqState,
    owner_pid: i32,
    queue_index: usize,
    flags: c_int,
) -> Result<c_int, c_int> {
    if state
        .descriptors
        .iter()
        .filter(|descriptor| descriptor.active && descriptor.owner_pid == owner_pid)
        .count()
        >= MAX_OPEN_PER_PROCESS
    {
        return Err(as_errno(Error::Mfile));
    }
    let slot = state
        .descriptors
        .iter()
        .position(|descriptor| !descriptor.active && descriptor.generation < HANDLE_GENERATION_MAX)
        .ok_or(as_errno(Error::Mfile))?;
    let generation = state.descriptors[slot].generation + 1;
    let token = (generation << HANDLE_SLOT_BITS) | (slot as u32 + 1);
    let queue = state
        .queues
        .get_mut(queue_index)
        .and_then(Option::as_mut)
        .ok_or(EBADF)?;
    queue.references = queue.references.checked_add(1).ok_or(ENOMEM)?;
    state.descriptors[slot] = MqDescriptor {
        generation,
        active: true,
        owner_pid,
        queue_index,
        readable: flags & O_ACCMODE != O_WRONLY,
        writable: flags & O_ACCMODE != O_RDONLY,
        nonblocking: flags & O_NONBLOCK != 0,
        close_on_exec: flags & O_CLOEXEC != 0,
    };
    Ok(token as c_int)
}

fn open_queue(
    process_id: i32,
    name: &[u8],
    flags: c_int,
    attributes: Option<MqAttr>,
) -> Result<c_int, c_int> {
    if flags & !(O_ACCMODE | O_CREAT | O_EXCL | O_NONBLOCK | O_CLOEXEC) != 0
        || flags & O_ACCMODE == O_ACCMODE
    {
        return Err(EINVAL);
    }
    let mut state = MQUEUE_STATE.lock();
    let existing = state
        .queues
        .iter()
        .position(|queue| queue.as_ref().is_some_and(|queue| !queue.unlinked && queue.name == name));
    let queue_index = match existing {
        Some(_index) if flags & (O_CREAT | O_EXCL) == (O_CREAT | O_EXCL) => {
            return Err(as_errno(Error::Exist));
        }
        Some(index) => index,
        None if flags & O_CREAT == 0 => return Err(as_errno(Error::NoEnt)),
        None => {
            let (max_messages, message_size) = attributes.map_or((10usize, 1024usize), |attr| {
                (attr.max_messages as usize, attr.message_size as usize)
            });
            if max_messages == 0
                || max_messages > MAX_MESSAGES
                || message_size == 0
                || message_size > MAX_MESSAGE_SIZE
            {
                return Err(EINVAL);
            }
            let mut queue_name = Vec::new();
            queue_name.try_reserve_exact(name.len()).map_err(|_| ENOMEM)?;
            queue_name.extend_from_slice(name);
            let queue = MessageQueue {
                name: queue_name,
                messages: Vec::new(),
                max_messages,
                message_size,
                waiting_receivers: 0,
                references: 0,
                unlinked: false,
                notification: None,
            };
            if let Some(index) = state.queues.iter().position(Option::is_none) {
                state.queues[index] = Some(queue);
                index
            } else {
                if state.queues.len() >= MAX_QUEUES {
                    return Err(ENOMEM);
                }
                state.queues.try_reserve(1).map_err(|_| ENOMEM)?;
                state.queues.push(Some(queue));
                state.queues.len() - 1
            }
        }
    };
    allocate_descriptor(&mut state, process_id, queue_index, flags)
}

// FIXME: The kernel has no message-queue objects, descriptors, or notification backend.
#[no_mangle]
pub unsafe extern "C" fn mq_close(descriptor: c_int) -> c_int {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let mut state = MQUEUE_STATE.lock();
    let (slot, queue_index) = match descriptor_parts(descriptor, process.pid(), &state) {
        Ok(parts) => parts,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    state.descriptors[slot].active = false;
    let queue = state.queues.get_mut(queue_index).and_then(Option::as_mut).unwrap();
    queue.references = queue.references.saturating_sub(1);
    if queue.references == 0 && queue.unlinked {
        state.queues[queue_index] = None;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn mq_getattr(descriptor: c_int, attributes: *mut c_void) -> c_int {
    if attributes.is_null() {
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
    let attr = {
        let state = MQUEUE_STATE.lock();
        let (slot, queue_index) = match descriptor_parts(descriptor, process.pid(), &state) {
            Ok(parts) => parts,
            Err(error) => {
                set_errno(error);
                return -1;
            }
        };
        let descriptor = state.descriptors[slot];
        let Some(queue) = state.queues.get(queue_index).and_then(Option::as_ref) else {
            set_errno(EBADF);
            return -1;
        };
        MqAttr {
            flags: if descriptor.nonblocking { O_NONBLOCK as c_long } else { 0 },
            max_messages: queue.max_messages as c_long,
            message_size: queue.message_size as c_long,
            current_messages: queue.messages.len() as c_long,
        }
    };
    match write_user_value(&process, attributes.cast(), &attr) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn mq_notify(descriptor: c_int, event: *const c_void) -> c_int {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let registration = if event.is_null() {
        None
    } else {
        let event = match read_user_value::<Sigevent>(&process, event.cast()) {
            Ok(event) => event,
            Err(error) => {
                set_errno(error);
                return -1;
            }
        };
        match event.notify {
            SIGEV_NONE => Some(MqNotification::None {
                process: Arc::clone(&process),
            }),
            SIGEV_SIGNAL if event.signal > 0 && event.signal as u32 <= u32::from(MAX_SIGNAL_NUMBER) => {
                Some(MqNotification::Signal {
                    process: Arc::clone(&process),
                    signal: event.signal as u8,
                    value: unsafe { event.value.pointer as usize },
                })
            }
            SIGEV_THREAD => {
                let Some(function) = event.function else {
                    set_errno(EINVAL);
                    return -1;
                };
                if !event.attributes.is_null() {
                    set_errno(EINVAL);
                    return -1;
                }
                Some(MqNotification::Thread {
                    process: Arc::clone(&process),
                    function,
                    value: unsafe { event.value.pointer as usize },
                })
            }
            _ => {
                set_errno(EINVAL);
                return -1;
            }
        }
    };
    let mut state = MQUEUE_STATE.lock();
    let (_, queue_index) = match descriptor_parts(descriptor, process.pid(), &state) {
        Ok(parts) => parts,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let Some(queue) = state.queues.get_mut(queue_index).and_then(Option::as_mut) else {
        set_errno(EBADF);
        return -1;
    };
    match (queue.notification.as_ref(), registration) {
        (Some(existing), None) => {
            let existing_pid = match existing {
                MqNotification::None { process }
                | MqNotification::Signal { process, .. }
                | MqNotification::Thread { process, .. } => process.pid(),
            };
            if existing_pid != process.pid() {
                set_errno(as_errno(Error::Busy));
                return -1;
            }
            queue.notification = None;
        }
        (Some(_), Some(_)) => {
            set_errno(as_errno(Error::Busy));
            return -1;
        }
        (None, registration) => queue.notification = registration,
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn mq_receive(
    descriptor: c_int,
    message: *mut c_char,
    message_size: usize,
    priority: *mut u32,
) -> isize {
    match receive_message(
        descriptor,
        message,
        message_size,
        priority,
        CLOCK_REALTIME,
        None,
    ) {
        Ok(size) => size,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn mq_send(
    descriptor: c_int,
    message: *const c_char,
    message_size: usize,
    priority: u32,
) -> c_int {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let message = match read_message_bytes(&process, message, message_size) {
        Ok(message) => message,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    match send_message(descriptor, message, priority, CLOCK_REALTIME, None) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn mq_setattr(
    descriptor: c_int,
    attributes: *const c_void,
    old_attributes: *mut c_void,
) -> c_int {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let requested = match read_user_value::<MqAttr>(&process, attributes.cast()) {
        Ok(attributes) => attributes,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    if requested.flags & !(O_NONBLOCK as c_long) != 0 {
        set_errno(EINVAL);
        return -1;
    }
    let mut state = MQUEUE_STATE.lock();
    let (slot, queue_index) = match descriptor_parts(descriptor, process.pid(), &state) {
        Ok(parts) => parts,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let Some(queue) = state.queues.get(queue_index).and_then(Option::as_ref) else {
        set_errno(EBADF);
        return -1;
    };
    if !old_attributes.is_null() {
        let old = MqAttr {
            flags: if state.descriptors[slot].nonblocking { O_NONBLOCK as c_long } else { 0 },
            max_messages: queue.max_messages as c_long,
            message_size: queue.message_size as c_long,
            current_messages: queue.messages.len() as c_long,
        };
        if let Err(error) = write_user_value(&process, old_attributes.cast(), &old) {
            set_errno(error);
            return -1;
        }
    }
    state.descriptors[slot].nonblocking = requested.flags & O_NONBLOCK as c_long != 0;
    0
}

#[no_mangle]
pub unsafe extern "C" fn mq_timedreceive(
    descriptor: c_int,
    message: *mut c_char,
    message_size: usize,
    priority: *mut u32,
    timeout: *const c_void,
) -> isize {
    let deadline = match absolute_timeout(CLOCK_REALTIME, timeout) {
        Ok(deadline) => deadline,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    match receive_message(
        descriptor,
        message,
        message_size,
        priority,
        CLOCK_REALTIME,
        deadline,
    ) {
        Ok(size) => size,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn mq_timedsend(
    descriptor: c_int,
    message: *const c_char,
    message_size: usize,
    priority: u32,
    timeout: *const c_void,
) -> c_int {
    let deadline = match absolute_timeout(CLOCK_REALTIME, timeout) {
        Ok(deadline) => deadline,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let message = match read_message_bytes(&process, message, message_size) {
        Ok(message) => message,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    match send_message(descriptor, message, priority, CLOCK_REALTIME, deadline) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn mq_unlink(name: *const c_char) -> c_int {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let name = match read_user_name(&process, name) {
        Ok(name) => name,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let mut state = MQUEUE_STATE.lock();
    let Some(index) = state.queues.iter().position(|queue| {
        queue.as_ref().is_some_and(|queue| !queue.unlinked && queue.name == name)
    }) else {
        set_errno(as_errno(Error::NoEnt));
        return -1;
    };
    let queue = state.queues[index].as_mut().unwrap();
    queue.unlinked = true;
    if queue.references == 0 {
        state.queues[index] = None;
    }
    0
}