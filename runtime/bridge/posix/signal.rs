/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;
use core::ffi::{c_char, c_int, c_long, c_void};
use core::mem::transmute;
use core::ptr::{read_volatile, write_volatile};

use crate::kernel::arch::traits::{CpuArchitecture as _, TimerArchitecture as _};
use crate::kernel::error::Error as KernelError;
use crate::kernel::process::Process;
use crate::kernel::scheduler::thread::ThreadManager;
use crate::kernel::signal::{
    SignalAction, SignalInfo, SignalSet, MAX_SIGNAL_NUMBER, SI_QUEUE, SIGCONT, SIGKILL, SIGSTOP,
};

use super::errno::{as_errno, EFAULT, EINVAL, ENOSYS};

const SIGNAL_SET_WORDS: usize = 16;
const SIG_BLOCK: c_int = 0;
const SIG_UNBLOCK: c_int = 1;
const SIG_SETMASK: c_int = 2;
const SA_SIGINFO: u32 = 0x0000_0004;
const SA_RESETHAND: u32 = 0x8000_0000;
const SA_RESTART: u32 = 0x1000_0000;
const SA_NODEFER: u32 = 0x4000_0000;
const SI_USER: i32 = 0;

#[repr(C)]
#[derive(Clone, Copy)]
pub union Sigval {
    integer: c_int,
    pointer: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CSignalSet {
    words: [u64; SIGNAL_SET_WORDS],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CSignalAction {
    handler: *mut c_void,
    mask: CSignalSet,
    flags: c_int,
    sigaction: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CSignalInfo {
    signal_number: c_int,
    code: c_int,
    error: c_int,
    sender_pid: i32,
    sender_uid: u32,
    address: *mut c_void,
    status: c_int,
    value: Sigval,
}

#[repr(C)]
struct CTimespec {
    seconds: c_long,
    nanoseconds: c_long,
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

fn signal_bit(signal_number: c_int) -> Option<(usize, u64)> {
    if signal_number <= 0 || signal_number as usize > MAX_SIGNAL_NUMBER as usize {
        None
    } else {
        let bit = signal_number as usize - 1;
        Some((bit / u64::BITS as usize, 1u64 << (bit % u64::BITS as usize)))
    }
}

fn set_kernel_errno(error: KernelError) {
    set_errno(as_errno(error));
}

fn current_process() -> Result<Arc<Process>, KernelError> {
    ThreadManager::current_process()
}

unsafe fn read_signal_set(set: *const c_void) -> SignalSet {
    SignalSet::from_bits(unsafe { read_volatile(set.cast::<u64>()) })
}

unsafe fn write_signal_set(set: *mut c_void, signals: SignalSet) {
    let words = set.cast::<u64>();
    for index in 0..SIGNAL_SET_WORDS {
        unsafe {
            write_volatile(
                words.add(index),
                if index == 0 { signals.bits() } else { 0 },
            )
        };
    }
}

fn monotonic_now_ns() -> u64 {
    crate::kernel::arch::TimerArchitecture::now_ns()
        .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns)
}

fn signal_info(information: SignalInfo) -> CSignalInfo {
    CSignalInfo {
        signal_number: information.signal_number as c_int,
        code: information.code,
        error: 0,
        sender_pid: information.sender_pid,
        sender_uid: information.sender_uid,
        address: core::ptr::null_mut(),
        status: information.status,
        value: Sigval {
            pointer: information.value as *mut c_void,
        },
    }
}

fn take_matching_signal(process: &Process, signals: SignalSet) -> Option<SignalInfo> {
    let information = ThreadManager::take_current_signal(signals)
        .or_else(|| process.take_pending_signal(signals));
    if information.is_some() {
        ThreadManager::take_current_signal_wake();
        ThreadManager::notify_process_signals(process.pid(), process.pending_signals());
    }
    information
}

fn default_signal_is_ignored(signal_number: u8) -> bool {
    matches!(signal_number, 17 | 23 | 28 | 29)
}

fn default_signal_stops_process(signal_number: u8) -> bool {
    matches!(signal_number, 19..=22)
}

pub(crate) fn dispatch_pending_signals() -> Option<bool> {
    let process = current_process().ok()?;
    if let Some(signal_number) = process.termination_signal() {
        ThreadManager::exit_current(signal_number as usize as *mut c_void);
    }
    ThreadManager::wait_if_process_stopped();
    let (_, blocked) = ThreadManager::current_signal_context().ok()?;
    let deliverable = SignalSet::ALL.difference(blocked);
    let mut restart = true;
    let mut handled = false;
    loop {
        let Some(information) = take_matching_signal(&process, deliverable) else {
            break;
        };
        let action = process.signal_action(information.signal_number).ok()?;
        if action.is_ignored() {
            continue;
        }
        if action.is_default() {
            if default_signal_is_ignored(information.signal_number) {
                continue;
            }
            if information.signal_number == SIGCONT {
                continue;
            }
            if default_signal_stops_process(information.signal_number) {
                process.stop_by_signal(information.signal_number);
                ThreadManager::wait_if_process_stopped();
                continue;
            }
            process.terminate_by_signal(information.signal_number);
            ThreadManager::exit_current(information.signal_number as usize as *mut c_void);
        }

        let previous_mask = ThreadManager::current_signal_context().ok()?.1;
        let handler_mask = previous_mask
            .union(action.mask)
            .union(if action.flags & SA_NODEFER == 0 {
                SignalSet::from_signal(information.signal_number)
            } else {
                SignalSet::EMPTY
            });
        let _ = ThreadManager::set_current_signal_mask(handler_mask);
        if action.flags & SA_RESETHAND != 0 {
            let _ = process.set_signal_action(information.signal_number, SignalAction::DEFAULT);
        }
        let mut information = signal_info(information);
        if action.flags & SA_SIGINFO != 0 {
            // SAFETY: sigaction stores this process's caller-provided C handler with the SA_SIGINFO ABI.
            let handler: unsafe extern "C" fn(c_int, *mut CSignalInfo, *mut c_void) =
                unsafe { transmute(action.handler) };
            // SAFETY: the handler was registered for this process and information is a live local C record.
            unsafe { handler(information.signal_number, &mut information, core::ptr::null_mut()) };
        } else {
            // SAFETY: sigaction stores this process's caller-provided C handler with the one-argument ABI.
            let handler: unsafe extern "C" fn(c_int) = unsafe { transmute(action.handler) };
            // SAFETY: the handler pointer is the valid callback registered by this process.
            unsafe { handler(information.signal_number) };
        }
        let _ = ThreadManager::set_current_signal_mask(previous_mask);
        ThreadManager::notify_process_signals(process.pid(), process.pending_signals());
        restart &= action.flags & SA_RESTART != 0;
        handled = true;
    }
    handled.then_some(restart)
}

pub(crate) fn current_signal_mask() -> Result<SignalSet, KernelError> {
    ThreadManager::current_signal_context().map(|(_, mask)| mask)
}

pub(crate) fn change_signal_mask(
    how: c_int,
    signals: SignalSet,
) -> Result<SignalSet, KernelError> {
    let previous = current_signal_mask()?;
    let updated = match how {
        SIG_BLOCK => previous.union(signals),
        SIG_UNBLOCK => previous.difference(signals),
        SIG_SETMASK => signals,
        _ => return Err(KernelError::Inval),
    };
    let previous = ThreadManager::set_current_signal_mask(updated)?;
    let process = current_process()?;
    ThreadManager::notify_process_signals(process.pid(), process.pending_signals());
    let _ = dispatch_pending_signals();
    Ok(previous)
}

fn wait_for_signal_set(
    process: &Process,
    signals: SignalSet,
    deadline_ns: Option<u64>,
) -> Result<SignalInfo, KernelError> {
    if signals.is_empty() {
        return Err(KernelError::Inval);
    }
    ThreadManager::set_current_signal_wait_set(Some(signals))?;
    loop {
        if let Some(information) = take_matching_signal(process, signals) {
            ThreadManager::set_current_signal_wait_set(None)?;
            return Ok(information);
        }
        if dispatch_pending_signals().is_some() {
            ThreadManager::set_current_signal_wait_set(None)?;
            return Err(KernelError::Intr);
        }
        if let Some(deadline_ns) = deadline_ns {
            if monotonic_now_ns() >= deadline_ns {
                ThreadManager::set_current_signal_wait_set(None)?;
                return Err(KernelError::Again);
            }
            let _ = ThreadManager::sleep_current_until(deadline_ns);
        } else if !ThreadManager::switch_current_for_signal()
            && !ThreadManager::take_current_signal_wake()
        {
            crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn sigaddset(set: *mut u8, signal_number: c_int) -> c_int {
    if set.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let Some((byte_index, bit)) = signal_bit(signal_number) else {
        set_errno(EINVAL);
        return -1;
    };
    unsafe {
        let word = set.cast::<u64>().add(byte_index);
        write_volatile(word, read_volatile(word) | bit);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn sigdelset(set: *mut u8, signal_number: c_int) -> c_int {
    if set.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let Some((byte_index, bit)) = signal_bit(signal_number) else {
        set_errno(EINVAL);
        return -1;
    };
    unsafe {
        let word = set.cast::<u64>().add(byte_index);
        write_volatile(word, read_volatile(word) & !bit);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn sigismember(set: *const u8, signal_number: c_int) -> c_int {
    if set.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let Some((byte_index, bit)) = signal_bit(signal_number) else {
        set_errno(EINVAL);
        return -1;
    };
    ((unsafe { read_volatile(set.cast::<u64>().add(byte_index)) } & bit) != 0) as c_int
}

fn signal_name(signal_number: c_int) -> Option<&'static [u8]> {
    Some(match signal_number {
        1 => b"SIGHUP\0",
        2 => b"SIGINT\0",
        3 => b"SIGQUIT\0",
        4 => b"SIGILL\0",
        5 => b"SIGTRAP\0",
        6 => b"SIGABRT\0",
        7 => b"SIGBUS\0",
        8 => b"SIGFPE\0",
        9 => b"SIGKILL\0",
        10 => b"SIGUSR1\0",
        11 => b"SIGSEGV\0",
        12 => b"SIGUSR2\0",
        13 => b"SIGPIPE\0",
        14 => b"SIGALRM\0",
        15 => b"SIGTERM\0",
        17 => b"SIGCHLD\0",
        18 => b"SIGCONT\0",
        19 => b"SIGSTOP\0",
        20 => b"SIGTSTP\0",
        21 => b"SIGTTIN\0",
        22 => b"SIGTTOU\0",
        23 => b"SIGURG\0",
        24 => b"SIGXCPU\0",
        25 => b"SIGXFSZ\0",
        26 => b"SIGVTALRM\0",
        27 => b"SIGPROF\0",
        28 => b"SIGWINCH\0",
        29 => b"SIGPOLL\0",
        31 => b"SIGSYS\0",
        _ => return None,
    })
}

#[no_mangle]
pub unsafe extern "C" fn sig2str(signal_number: c_int, output: *mut c_char) -> c_int {
    let Some(name) = signal_name(signal_number) else {
        return EINVAL;
    };
    if output.is_null() {
        return EFAULT;
    }
    for (index, byte) in name.iter().copied().enumerate() {
        unsafe { output.add(index).write(byte as c_char) };
    }
    0
}

unsafe fn string_equals(input: *const c_char, expected: &[u8]) -> bool {
    for (index, byte) in expected.iter().copied().enumerate() {
        if unsafe { read_volatile(input.cast::<u8>().add(index)) } != byte {
            return false;
        }
    }
    true
}

#[no_mangle]
pub unsafe extern "C" fn str2sig(input: *const c_char, signal_number: *mut c_int) -> c_int {
    if input.is_null() || signal_number.is_null() {
        return EFAULT;
    }
    for value in 1..=31 {
        if let Some(name) = signal_name(value) {
            if unsafe { string_equals(input, name) } {
                unsafe { signal_number.write(value) };
                return 0;
            }
        }
    }
    EINVAL
}

#[no_mangle]
pub unsafe extern "C" fn kill(process_id: i32, signal_number: c_int) -> c_int {
    if signal_number < 0 || signal_number > MAX_SIGNAL_NUMBER as c_int {
        set_errno(EINVAL);
        return -1;
    }
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    let ids = process.user_ids();
    let information = SignalInfo::new(
        signal_number as u8,
        SI_USER,
        process.pid(),
        ids[1],
        0,
        0,
    );
    match process.send_signal(process_id, signal_number, information) {
        Ok(()) => {
            if signal_number != 0
                && (process_id == process.pid()
                    || process_id == 0
                    || process_id < -1
                        && process_id.checked_neg() == Some(process.process_group_id()))
            {
                let _ = dispatch_pending_signals();
            }
            0
        }
        Err(error) => {
            set_kernel_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn raise(signal_number: c_int) -> c_int {
    if signal_number <= 0 || signal_number > MAX_SIGNAL_NUMBER as c_int {
        set_errno(EINVAL);
        return -1;
    }
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    let Ok((thread_id, _)) = ThreadManager::current_signal_context() else {
        set_errno(EINVAL);
        return -1;
    };
    if signal_number as u8 == SIGKILL {
        process.terminate_by_signal(SIGKILL);
        ThreadManager::exit_current(SIGKILL as usize as *mut c_void);
    }
    if signal_number as u8 == SIGSTOP {
        process.stop_by_signal(SIGSTOP);
        ThreadManager::wait_if_process_stopped();
        return 0;
    }
    if signal_number as u8 == SIGCONT {
        process.continue_by_signal();
    }
    let ids = process.user_ids();
    let information = SignalInfo::new(
        signal_number as u8,
        SI_USER,
        process.pid(),
        ids[1],
        0,
        0,
    );
    match ThreadManager::queue_signal_by_id(thread_id, information) {
        Ok(()) => {
            let _ = dispatch_pending_signals();
            0
        }
        Err(error) => {
            set_kernel_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn sigpending(set: *mut c_void) -> c_int {
    if set.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    let mask = match current_signal_mask() {
        Ok(mask) => mask,
        Err(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    let thread_pending = ThreadManager::pending_current_signals().unwrap_or(SignalSet::EMPTY);
    let pending = process
        .pending_signals()
        .union(thread_pending)
        .intersection(mask);
    unsafe { write_signal_set(set, pending) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn sigprocmask(
    how: c_int,
    set: *const c_void,
    old_set: *mut c_void,
) -> c_int {
    let previous = match current_signal_mask() {
        Ok(mask) => mask,
        Err(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    if !old_set.is_null() {
        unsafe { write_signal_set(old_set, previous) };
    }
    if !set.is_null() {
        let signals = unsafe { read_signal_set(set) };
        if let Err(error) = change_signal_mask(how, signals) {
            set_kernel_errno(error);
            return -1;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn sigqueue(
    process_id: i32,
    signal_number: c_int,
    value: Sigval,
) -> c_int {
    if signal_number < 0 || signal_number > MAX_SIGNAL_NUMBER as c_int {
        set_errno(EINVAL);
        return -1;
    }
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    let ids = process.user_ids();
    let information = SignalInfo::new(
        signal_number as u8,
        SI_QUEUE,
        process.pid(),
        ids[1],
        unsafe { value.pointer } as usize,
        0,
    );
    match process.send_signal(process_id, signal_number, information) {
        Ok(()) => {
            if signal_number != 0
                && (process_id == process.pid()
                    || process_id == 0
                    || process_id < -1
                        && process_id.checked_neg() == Some(process.process_group_id()))
            {
                let _ = dispatch_pending_signals();
            }
            0
        }
        Err(error) => {
            set_kernel_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn sigsuspend(set: *const c_void) -> c_int {
    if set.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let Ok(previous_mask) = current_signal_mask() else {
        set_errno(EINVAL);
        return -1;
    };
    let temporary_mask = unsafe { read_signal_set(set) }.without_uncatchable();
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    if let Err(error) = ThreadManager::set_current_signal_mask(temporary_mask) {
        set_kernel_errno(error);
        return -1;
    }
    ThreadManager::notify_process_signals(process.pid(), process.pending_signals());
    let wait_set = SignalSet::ALL.difference(temporary_mask);
    if let Err(error) = ThreadManager::set_current_signal_wait_set(Some(wait_set)) {
        let _ = ThreadManager::set_current_signal_mask(previous_mask);
        set_kernel_errno(error);
        return -1;
    }
    loop {
        if dispatch_pending_signals().is_some() {
            let _ = ThreadManager::set_current_signal_wait_set(None);
            let _ = ThreadManager::set_current_signal_mask(previous_mask);
            set_errno(as_errno(KernelError::Intr));
            return -1;
        }
        if !ThreadManager::switch_current_for_signal()
            && !ThreadManager::take_current_signal_wake()
        {
            crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn sigtimedwait(
    set: *const c_void,
    information: *mut c_void,
    timeout: *const c_void,
) -> c_int {
    if set.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let signals = unsafe { read_signal_set(set) };
    let deadline_ns = if timeout.is_null() {
        None
    } else {
        let timeout = unsafe { read_volatile(timeout.cast::<CTimespec>()) };
        if timeout.seconds < 0
            || timeout.nanoseconds < 0
            || timeout.nanoseconds >= 1_000_000_000
        {
            set_errno(EINVAL);
            return -1;
        }
        let duration = (timeout.seconds as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(timeout.nanoseconds as u64);
        Some(monotonic_now_ns().saturating_add(duration))
    };
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    match wait_for_signal_set(&process, signals, deadline_ns) {
        Ok(signal) => {
            if !information.is_null() {
                unsafe { write_volatile(information.cast::<CSignalInfo>(), signal_info(signal)) };
            }
            signal.signal_number as c_int
        }
        Err(error) => {
            set_kernel_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn sigwait(set: *const c_void, signal_number: *mut c_int) -> c_int {
    if set.is_null() || signal_number.is_null() {
        return as_errno(KernelError::Fault);
    }
    let signals = unsafe { read_signal_set(set) };
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => return as_errno(error),
    };
    match wait_for_signal_set(&process, signals, None) {
        Ok(information) => {
            unsafe { write_volatile(signal_number, information.signal_number as c_int) };
            0
        }
        Err(error) => as_errno(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn sigwaitinfo(set: *const c_void, information: *mut c_void) -> c_int {
    unsafe { sigtimedwait(set, information, core::ptr::null()) }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_kill(thread: usize, signal_number: c_int) -> c_int {
    if signal_number < 0 || signal_number > MAX_SIGNAL_NUMBER as c_int {
        return as_errno(KernelError::Inval);
    }
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => return as_errno(error),
    };
    if !ThreadManager::thread_belongs_to_process(thread, process.pid()) {
        return as_errno(KernelError::Srch);
    }
    if signal_number == 0 {
        return 0;
    }
    if signal_number as u8 == SIGKILL {
        process.terminate_by_signal(SIGKILL);
        ThreadManager::exit_current(SIGKILL as usize as *mut c_void);
    }
    if signal_number as u8 == SIGSTOP {
        process.stop_by_signal(SIGSTOP);
        ThreadManager::wait_if_process_stopped();
        return 0;
    }
    if signal_number as u8 == SIGCONT {
        process.continue_by_signal();
    }
    let ids = process.user_ids();
    let information = SignalInfo::new(
        signal_number as u8,
        SI_USER,
        process.pid(),
        ids[1],
        0,
        0,
    );
    match ThreadManager::queue_signal_by_id(thread, information) {
        Ok(()) => {
            if ThreadManager::current_thread_id() == thread {
                let _ = dispatch_pending_signals();
            }
            0
        }
        Err(error) => as_errno(error),
    }
}

#[no_mangle]
pub unsafe extern "C" fn sigaction(
    signal_number: c_int,
    action: *const CSignalAction,
    old_action: *mut CSignalAction,
) -> c_int {
    let Some(signal_number) = u8::try_from(signal_number)
        .ok()
        .filter(|number| *number != 0 && *number <= MAX_SIGNAL_NUMBER)
    else {
        set_errno(EINVAL);
        return -1;
    };
    let process = match current_process() {
        Ok(process) => process,
        Err(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    let replacement = if action.is_null() {
        None
    } else {
        let action = unsafe { read_volatile(action) };
        let flags = action.flags as u32;
        let handler = if flags & SA_SIGINFO != 0 {
            action.sigaction
        } else {
            action.handler
        };
        Some(SignalAction {
            handler: handler as usize,
            mask: SignalSet::from_bits(action.mask.words[0]),
            flags,
        })
    };
    let previous = match process.signal_action(signal_number) {
        Ok(previous) => previous,
        Err(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    if let Some(replacement) = replacement {
        if let Err(error) = process.set_signal_action(signal_number, replacement) {
            set_kernel_errno(error);
            return -1;
        }
    }
    if !old_action.is_null() {
        let handler = previous.handler as *mut c_void;
        unsafe {
            write_volatile(
                old_action,
                CSignalAction {
                    handler,
                    mask: CSignalSet {
                        words: [previous.mask.bits(), 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                    },
                    flags: previous.flags as c_int,
                    sigaction: handler,
                },
            )
        };
    }
    if replacement.is_some() {
        let _ = dispatch_pending_signals();
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn psignal(_signal_number: c_int, _message: *const c_char) {}

#[no_mangle]
pub unsafe extern "C" fn psiginfo(_information: *const c_void, _message: *const c_char) {}

#[no_mangle]
pub unsafe extern "C" fn signal(
    signal_number: c_int,
    handler: *mut c_void,
) -> *mut c_void {
    let mut action = CSignalAction {
        handler,
        mask: CSignalSet {
            words: [0; SIGNAL_SET_WORDS],
        },
        flags: SA_RESTART as c_int,
        sigaction: handler,
    };
    let mut previous = CSignalAction {
        handler: core::ptr::null_mut(),
        mask: CSignalSet {
            words: [0; SIGNAL_SET_WORDS],
        },
        flags: 0,
        sigaction: core::ptr::null_mut(),
    };
    if unsafe { sigaction(signal_number, &mut action, &mut previous) } == 0 {
        previous.handler
    } else {
        usize::MAX as *mut c_void
    }
}

#[no_mangle]
pub unsafe extern "C" fn sigaltstack(_stack: *const c_void, _old_stack: *mut c_void) -> c_int {
    set_errno(ENOSYS);
    -1
}