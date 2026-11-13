/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::{c_int, c_void};
use core::mem::{size_of, MaybeUninit};

use crate::kernel::arch::traits::TimerArchitecture as _;
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::process::Process;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::scheduler::thread::{ThreadManager, ThreadStack};

use super::errno::{as_errno, EAGAIN, EFAULT, EINVAL, ENOMEM};

const MAX_AIO_REQUESTS: usize = 32;
const MAX_AIO_SIZE: usize = 1 << 20;
const EINPROGRESS: c_int = 115;
const ECANCELED: c_int = 125;
const ETIMEDOUT: c_int = 110;
const CLOCK_REALTIME: c_int = 0;
const CLOCK_MONOTONIC: c_int = 1;
const SIGEV_SIGNAL: c_int = 0;
const SIGEV_NONE: c_int = 1;
const SIGEV_THREAD: c_int = 2;
const LIO_READ: c_int = 0;
const LIO_WRITE: c_int = 1;
const LIO_NOP: c_int = 2;
const LIO_WAIT: c_int = 0;
const LIO_NOWAIT: c_int = 1;
const O_DSYNC: c_int = 0o10000;
const O_SYNC: c_int = 0o4010000;
const AIO_CANCELED: c_int = 0;
const AIO_NOTCANCELED: c_int = 1;
const AIO_ALLDONE: c_int = 2;

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
struct Timespec {
    seconds: i64,
    nanoseconds: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Aiocb {
    file_descriptor: c_int,
    offset: i64,
    buffer: *mut c_void,
    byte_count: usize,
    request_priority: c_int,
    event: Sigevent,
    list_opcode: c_int,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Operation {
    Read,
    Write,
    Fsync,
}

#[derive(Clone, Copy)]
enum Notification {
    None,
    Signal { signal: u8, value: usize },
    Thread {
        function: unsafe extern "C" fn(Sigval),
        value: usize,
    },
}

struct NotificationContext {
    function: unsafe extern "C" fn(Sigval),
    value: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum RequestState {
    Idle,
    Queued,
    Running,
    Complete,
}

#[derive(Clone, Copy)]
struct AioRequest {
    generation: u32,
    active: bool,
    owner_pid: i32,
    control_block: usize,
    file_descriptor: c_int,
    offset: i64,
    buffer: usize,
    byte_count: usize,
    operation: Operation,
    state: RequestState,
    cancelled: bool,
    result: isize,
    error: c_int,
    notification: Notification,
}

impl AioRequest {
    const EMPTY: Self = Self {
        generation: 0,
        active: false,
        owner_pid: 0,
        control_block: 0,
        file_descriptor: -1,
        offset: 0,
        buffer: 0,
        byte_count: 0,
        operation: Operation::Read,
        state: RequestState::Idle,
        cancelled: false,
        result: -1,
        error: 0,
        notification: Notification::None,
    };
}

struct WorkerContext {
    slot: usize,
    generation: u32,
}

static AIO_REQUESTS: IrqSpinLock<[AioRequest; MAX_AIO_REQUESTS]> =
    IrqSpinLock::new([AioRequest::EMPTY; MAX_AIO_REQUESTS]);

unsafe extern "C" {
    fn clock_gettime(clock_id: c_int, output: *mut Timespec) -> c_int;
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
    process.address_space().read(pointer as usize, bytes).map_err(as_errno)?;
    Ok(unsafe { value.assume_init() })
}

fn write_user_value<T: Copy>(process: &Process, pointer: *mut T, value: &T) -> Result<(), c_int> {
    if pointer.is_null() {
        return Ok(());
    }
    let bytes = unsafe { core::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) };
    process.address_space().write(pointer as usize, bytes).map_err(as_errno)
}

fn current_process() -> Result<alloc::sync::Arc<Process>, c_int> {
    ThreadManager::current_process().map_err(as_errno)
}

fn read_notification(event: Sigevent) -> Result<Notification, c_int> {
    match event.notify {
        SIGEV_NONE => Ok(Notification::None),
        SIGEV_SIGNAL if event.signal == 0 => Ok(Notification::None),
        SIGEV_SIGNAL if event.signal > 0 && event.signal <= 63 => Ok(Notification::Signal {
            signal: event.signal as u8,
            value: unsafe { event.value.pointer as usize },
        }),
        SIGEV_THREAD => {
            let Some(function) = event.function else {
                return Err(EINVAL);
            };
            if !event.attributes.is_null() {
                return Err(EINVAL);
            }
            Ok(Notification::Thread {
                function,
                value: unsafe { event.value.pointer as usize },
            })
        }
        _ => Err(EINVAL),
    }
}

fn find_request(requests: &[AioRequest; MAX_AIO_REQUESTS], pid: i32, control: usize) -> Option<usize> {
    requests
        .iter()
        .position(|request| request.active && request.owner_pid == pid && request.control_block == control)
}

fn request_status(pid: i32, control: usize) -> Result<AioRequest, c_int> {
    let requests = AIO_REQUESTS.lock();
    let index = find_request(&requests, pid, control).ok_or(EINVAL)?;
    Ok(requests[index])
}

unsafe extern "C" fn notification_thread(argument: *mut c_void) -> *mut c_void {
    let context = unsafe { Box::from_raw(argument.cast::<NotificationContext>()) };
    unsafe { (context.function)(Sigval { pointer: context.value as *mut c_void }) };
    core::ptr::null_mut()
}

fn notify_completion(process: alloc::sync::Arc<Process>, notification: Notification) {
    match notification {
        Notification::None => {}
        Notification::Signal { signal, value } => {
            let ids = process.user_ids();
            let _ = process.queue_signal(crate::kernel::signal::SignalInfo::new(
                signal,
                crate::kernel::signal::SI_QUEUE,
                0,
                ids[1],
                value,
                0,
            ));
        }
        Notification::Thread { function, value } => {
            let argument = Box::into_raw(Box::new(NotificationContext { function, value }))
                .cast::<c_void>();
            if ThreadManager::create_thread_in_process(
                process,
                ThreadStack::DEFAULT_SIZE,
                notification_thread,
                argument,
                true,
            )
            .is_err()
            {
                unsafe { drop(Box::from_raw(argument.cast::<NotificationContext>())) };
            }
        }
    }
}

unsafe extern "C" fn aio_worker(argument: *mut c_void) -> *mut c_void {
    let context = unsafe { Box::from_raw(argument.cast::<WorkerContext>()) };
    let process = match current_process() {
        Ok(process) => process,
        Err(_) => return core::ptr::null_mut(),
    };
    let (request, cancelled) = {
        let mut requests = AIO_REQUESTS.lock();
        let Some(request) = requests.get_mut(context.slot) else {
            return core::ptr::null_mut();
        };
        if !request.active || request.generation != context.generation {
            return core::ptr::null_mut();
        }
        if request.cancelled {
            request.state = RequestState::Complete;
            request.result = -1;
            request.error = ECANCELED;
            (*request, true)
        } else {
            request.state = RequestState::Running;
            (*request, false)
        }
    };
    if cancelled {
        notify_completion(process, request.notification);
        return core::ptr::null_mut();
    }

    let completion = match request.operation {
        Operation::Read => {
            let mut buffer = Vec::new();
            if buffer.try_reserve_exact(request.byte_count).is_err() {
                Err(ENOMEM)
            } else {
                buffer.resize(request.byte_count, 0);
                match crate::kernel::runtime::runtime_libc::filesystem::positioned_read(
                    request.file_descriptor,
                    buffer.as_mut_ptr(),
                    request.byte_count,
                    request.offset,
                ) {
                    Ok(amount) => process
                        .address_space()
                        .write(request.buffer, &buffer[..amount])
                        .map(|()| amount as isize)
                        .map_err(as_errno),
                    Err(error) => Err(error),
                }
            }
        }
        Operation::Write => {
            let mut buffer = Vec::new();
            if buffer.try_reserve_exact(request.byte_count).is_err() {
                Err(ENOMEM)
            } else {
                buffer.resize(request.byte_count, 0);
                process
                    .address_space()
                    .read(request.buffer, &mut buffer)
                    .map_err(as_errno)
                    .and_then(|()| {
                        crate::kernel::runtime::runtime_libc::filesystem::positioned_write(
                            request.file_descriptor,
                            buffer.as_ptr(),
                            request.byte_count,
                            request.offset,
                        )
                        .map(|amount| amount as isize)
                    })
            }
        }
        Operation::Fsync => Ok(0),
    };
    let (result, error, notification) = match completion {
        Ok(result) => (result, 0, request.notification),
        Err(error) => (-1, error.unsigned_abs() as c_int, request.notification),
    };
    {
        let mut requests = AIO_REQUESTS.lock();
        if let Some(current) = requests.get_mut(context.slot) {
            if current.active && current.generation == context.generation {
                current.state = RequestState::Complete;
                current.result = result;
                current.error = error;
            }
        }
    }
    notify_completion(process, notification);
    core::ptr::null_mut()
}

fn start_request(control_block: *mut c_void, operation: Operation) -> Result<(), c_int> {
    if control_block.is_null() {
        return Err(EINVAL);
    }
    let process = current_process()?;
    let control = read_user_value::<Aiocb>(&process, control_block.cast())?;
    if control.request_priority != 0 || control.byte_count > MAX_AIO_SIZE {
        return Err(EINVAL);
    }
    if control.byte_count != 0 {
        if control.buffer.is_null()
            || !process.address_space().user_range_accessible(
                control.buffer as usize,
                control.byte_count,
                operation == Operation::Read,
            )
        {
            return Err(EFAULT);
        }
    }
    let notification = read_notification(control.event)?;
    let (slot, generation) = {
        let mut requests = AIO_REQUESTS.lock();
        if find_request(&requests, process.pid(), control_block as usize).is_some() {
            return Err(as_errno(Error::Busy));
        }
        let slot = requests
            .iter()
            .position(|request| !request.active && request.generation < u32::MAX)
            .ok_or(ENOMEM)?;
        let generation = requests[slot].generation + 1;
        requests[slot] = AioRequest {
            generation,
            active: true,
            owner_pid: process.pid(),
            control_block: control_block as usize,
            file_descriptor: control.file_descriptor,
            offset: control.offset,
            buffer: control.buffer as usize,
            byte_count: control.byte_count,
            operation,
            state: RequestState::Queued,
            cancelled: false,
            result: -1,
            error: EINPROGRESS,
            notification,
        };
        (slot, generation)
    };
    let worker_context = Box::into_raw(Box::new(WorkerContext { slot, generation }))
        .cast::<c_void>();
    if let Err(error) = ThreadManager::create_thread_in_process(
        process,
        ThreadStack::DEFAULT_SIZE,
        aio_worker,
        worker_context,
        true,
    ) {
        unsafe { drop(Box::from_raw(worker_context.cast::<WorkerContext>())) };
        let mut requests = AIO_REQUESTS.lock();
        requests[slot].active = false;
        return Err(as_errno(error));
    }
    Ok(())
}

fn wait_for_requests(list: &[usize], timeout: Option<(i128, c_int)>) -> Result<(), c_int> {
    loop {
        ThreadManager::cancel_current_if_requested();
        let pid = current_process()?.pid();
        {
            let requests = AIO_REQUESTS.lock();
            if list.iter().any(|control| {
                find_request(&requests, pid, *control)
                    .is_some_and(|index| requests[index].state == RequestState::Complete)
            }) {
                return Ok(());
            }
        }
        if let Some((deadline_ns, clock_id)) = timeout {
            let mut now = Timespec { seconds: 0, nanoseconds: 0 };
            if unsafe { clock_gettime(clock_id, &mut now) } != 0 {
                return Err(unsafe { crate::kernel::scheduler::thread::__errno_location().read() });
            }
            let now_ns = i128::from(now.seconds) * 1_000_000_000 + i128::from(now.nanoseconds);
            if now_ns >= deadline_ns {
                return Err(EAGAIN);
            }
        }
        if crate::kernel::runtime::runtime_posix::signal::dispatch_pending_signals()
            .is_some_and(|restart| !restart)
        {
            return Err(as_errno(Error::Intr));
        }
        let _ = ThreadManager::switch_current(crate::kernel::scheduler::thread::ThreadState::Ready);
    }
}

#[no_mangle]
pub unsafe extern "C" fn aio_cancel(
    file_descriptor: c_int,
    control_block: *mut c_void,
) -> c_int {
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let mut requests = AIO_REQUESTS.lock();
    let mut found = false;
    let mut running = false;
    let mut cancelled = false;
    for request in &mut *requests {
        if !request.active
            || request.owner_pid != process.pid()
            || request.file_descriptor != file_descriptor
            || (!control_block.is_null() && request.control_block != control_block as usize)
        {
            continue;
        }
        found = true;
        match request.state {
            RequestState::Queued => {
                request.cancelled = true;
                cancelled = true;
            }
            RequestState::Running => running = true,
            RequestState::Complete | RequestState::Idle => {}
        }
    }
    if !found || (!cancelled && !running) {
        AIO_ALLDONE
    } else if running {
        AIO_NOTCANCELED
    } else {
        AIO_CANCELED
    }
}

#[no_mangle]
pub unsafe extern "C" fn aio_error(control_block: *const c_void) -> c_int {
    if control_block.is_null() {
        return EINVAL;
    }
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => return error,
    };
    match request_status(process.pid(), control_block as usize) {
        Ok(request) if request.state == RequestState::Complete => request.error,
        Ok(_) => EINPROGRESS,
        Err(error) => error,
    }
}

#[no_mangle]
pub unsafe extern "C" fn aio_return(control_block: *mut c_void) -> isize {
    if control_block.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let mut requests = AIO_REQUESTS.lock();
    let Some(index) = find_request(&requests, process.pid(), control_block as usize) else {
        set_errno(EINVAL);
        return -1;
    };
    if requests[index].state != RequestState::Complete {
        set_errno(EINVAL);
        return -1;
    }
    let result = requests[index].result;
    let error = requests[index].error;
    requests[index].active = false;
    if result < 0 && error != 0 {
        set_errno(error);
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn aio_read(control_block: *mut c_void) -> c_int {
    match start_request(control_block, Operation::Read) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn aio_write(control_block: *mut c_void) -> c_int {
    match start_request(control_block, Operation::Write) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn aio_fsync(operation: c_int, control_block: *mut c_void) -> c_int {
    if !matches!(operation, O_DSYNC | O_SYNC) {
        set_errno(EINVAL);
        return -1;
    }
    match start_request(control_block, Operation::Fsync) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn aio_suspend(
    list: *const *const c_void,
    count: c_int,
    timeout: *const c_void,
) -> c_int {
    if list.is_null() || count <= 0 || count as usize > MAX_AIO_REQUESTS {
        set_errno(EINVAL);
        return -1;
    }
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let mut controls = Vec::new();
    if controls.try_reserve_exact(count as usize).is_err() {
        set_errno(ENOMEM);
        return -1;
    }
    for index in 0..count as usize {
        let address = (list as usize).saturating_add(index * size_of::<*const c_void>());
        let mut pointer = MaybeUninit::<*const c_void>::uninit();
        let bytes = unsafe {
            core::slice::from_raw_parts_mut(pointer.as_mut_ptr().cast::<u8>(), size_of::<*const c_void>())
        };
        if let Err(error) = process.address_space().read(address, bytes) {
            set_errno(as_errno(error));
            return -1;
        }
        let control = unsafe { pointer.assume_init() };
        if !control.is_null() {
            controls.push(control as usize);
        }
    }
    if controls.is_empty() {
        set_errno(EINVAL);
        return -1;
    }
    let requests = AIO_REQUESTS.lock();
    let has_active_request = controls.iter().any(|control| {
        find_request(&requests, process.pid(), *control).is_some()
    });
    drop(requests);
    if !has_active_request {
        set_errno(EINVAL);
        return -1;
    }
    let timeout = if timeout.is_null() {
        None
    } else {
        let timeout = match read_user_value::<Timespec>(&process, timeout.cast()) {
            Ok(timeout) => timeout,
            Err(error) => {
                set_errno(error);
                return -1;
            }
        };
        if timeout.seconds < 0 || !(0..1_000_000_000).contains(&timeout.nanoseconds) {
            set_errno(EINVAL);
            return -1;
        }
        let now_ns = crate::kernel::arch::TimerArchitecture::now_ns()
            .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns);
        Some((
            i128::from(now_ns)
                + i128::from(timeout.seconds) * 1_000_000_000
                + i128::from(timeout.nanoseconds),
            CLOCK_MONOTONIC,
        ))
    };
    match wait_for_requests(&controls, timeout) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn lio_listio(
    mode: c_int,
    list: *const *mut c_void,
    count: c_int,
    _event: *mut c_void,
) -> c_int {
    if !matches!(mode, LIO_WAIT | LIO_NOWAIT)
        || count < 0
        || count as usize > MAX_AIO_REQUESTS
        || (count != 0 && list.is_null())
    {
        set_errno(EINVAL);
        return -1;
    }
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(error);
            return -1;
        }
    };
    let mut controls = Vec::new();
    if controls.try_reserve_exact(count as usize).is_err() {
        set_errno(ENOMEM);
        return -1;
    }
    for index in 0..count as usize {
        let address = (list as usize).saturating_add(index * size_of::<*mut c_void>());
        let mut pointer = MaybeUninit::<*mut c_void>::uninit();
        let bytes = unsafe {
            core::slice::from_raw_parts_mut(pointer.as_mut_ptr().cast::<u8>(), size_of::<*mut c_void>())
        };
        if let Err(error) = process.address_space().read(address, bytes) {
            set_errno(as_errno(error));
            return -1;
        }
        let control = unsafe { pointer.assume_init() };
        if control.is_null() {
            continue;
        }
        let request = match read_user_value::<Aiocb>(&process, control.cast()) {
            Ok(request) => request,
            Err(error) => {
                set_errno(error);
                return -1;
            }
        };
        let operation = match request.list_opcode {
            LIO_NOP => continue,
            LIO_READ => Operation::Read,
            LIO_WRITE => Operation::Write,
            _ => {
                set_errno(EINVAL);
                return -1;
            }
        };
        if let Err(error) = start_request(control, operation) {
            set_errno(error);
            return -1;
        }
        controls.push(control as usize);
    }
    if mode == LIO_WAIT && !controls.is_empty() {
        if let Err(error) = wait_for_requests(&controls, None) {
            set_errno(error);
            return -1;
        }
    }
    0
}