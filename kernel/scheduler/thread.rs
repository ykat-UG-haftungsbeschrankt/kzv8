/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::c_void;
use core::ptr::addr_of_mut;
use core::slice::from_raw_parts_mut;
use core::sync::atomic::{AtomicUsize, Ordering};

const MAX_THREADS: usize = 8;
const MAX_TLS_KEYS: usize = 64;
const STACK_SIZE: usize = 16 * 1024;
const MAIN_THREAD_ID: usize = 1;

const EAGAIN: i32 = 11;
const EBUSY: i32 = 16;
const EDEADLK: i32 = 35;
const EINVAL: i32 = 22;
const EPERM: i32 = 1;
const ESRCH: i32 = 3;
const ENOSYS: i32 = 38;

const JOINABLE: i32 = 0;
const DETACHED: i32 = 1;

type StartRoutine = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

#[repr(C)]
struct ThreadAttributes {
    stack_size: usize,
    detach_state: i32,
    stack_address: *mut c_void,
}

#[repr(C)]
struct PthreadMutex {
    owner: AtomicUsize,
}

#[repr(C)]
struct PthreadCondition {
    generation: AtomicUsize,
}

#[derive(Clone, Copy)]
struct TlsKey {
    active: bool,
    destructor: Option<unsafe extern "C" fn(*mut c_void)>,
}

impl TlsKey {
    const EMPTY: Self = Self {
        active: false,
        destructor: None,
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ThreadState {
    Unused,
    Running,
    Ready,
    Blocked,
    Exited,
}

#[derive(Clone, Copy)]
struct Thread {
    id: usize,
    stack_pointer: usize,
    state: ThreadState,
    start_routine: Option<StartRoutine>,
    argument: *mut c_void,
    result: *mut c_void,
    joiner: usize,
    detached: bool,
    errno: i32,
    tls: [*mut c_void; MAX_TLS_KEYS],
    #[cfg(target_arch = "x86_64")]
    fpu_context: crate::arch::FpuContext,
    #[cfg(target_arch = "aarch64")]
    fpu_context: crate::arch::FpuContext,
}

impl Thread {
    const EMPTY: Self = Self {
        id: 0,
        stack_pointer: 0,
        state: ThreadState::Unused,
        start_routine: None,
        argument: core::ptr::null_mut(),
        result: core::ptr::null_mut(),
        joiner: 0,
        detached: false,
        errno: 0,
        tls: [core::ptr::null_mut(); MAX_TLS_KEYS],
        #[cfg(target_arch = "x86_64")]
        fpu_context: crate::arch::FpuContext::INITIAL,
        #[cfg(target_arch = "aarch64")]
        fpu_context: crate::arch::FpuContext::INITIAL,
    };
}

#[derive(Clone, Copy)]
#[repr(align(16))]
struct ThreadStack([u8; STACK_SIZE]);

static mut THREADS: [Thread; MAX_THREADS] = [Thread::EMPTY; MAX_THREADS];
static mut TLS_KEYS: [TlsKey; MAX_TLS_KEYS] = [TlsKey::EMPTY; MAX_TLS_KEYS];
static mut STACKS: [ThreadStack; MAX_THREADS] = [ThreadStack([0; STACK_SIZE]); MAX_THREADS];
static mut CURRENT_SLOT: usize = 0;
static mut NEXT_THREAD_ID: usize = MAIN_THREAD_ID + 1;
static mut INITIALIZED: bool = false;
static mut FALLBACK_ERRNO: i32 = 0;

pub(crate) fn initialize() {
    let interrupt_state = crate::arch::interrupts::disable();
    unsafe {
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        for index in 0..MAX_THREADS {
            threads.add(index).write(Thread::EMPTY);
        }
        let keys = addr_of_mut!(TLS_KEYS).cast::<TlsKey>();
        for index in 0..MAX_TLS_KEYS {
            keys.add(index).write(TlsKey::EMPTY);
        }
        threads.add(0).write(Thread {
            id: MAIN_THREAD_ID,
            state: ThreadState::Running,
            ..Thread::EMPTY
        });
        CURRENT_SLOT = 0;
        NEXT_THREAD_ID = MAIN_THREAD_ID + 1;
        INITIALIZED = true;
    }
    crate::arch::interrupts::restore(interrupt_state);
}

#[no_mangle]
pub extern "C" fn __errno_location() -> *mut i32 {
    unsafe {
        if INITIALIZED {
            let current = CURRENT_SLOT;
            let threads = addr_of_mut!(THREADS).cast::<Thread>();
            addr_of_mut!((*threads.add(current)).errno)
        } else {
            addr_of_mut!(FALLBACK_ERRNO)
        }
    }
}

unsafe fn find_thread(id: usize) -> Option<usize> {
    let threads = addr_of_mut!(THREADS).cast::<Thread>();
    for index in 0..MAX_THREADS {
        let thread = threads.add(index);
        if (*thread).state != ThreadState::Unused && (*thread).id == id {
            return Some(index);
        }
    }
    None
}

unsafe fn find_ready(current: usize) -> Option<usize> {
    let threads = addr_of_mut!(THREADS).cast::<Thread>();
    for distance in 1..=MAX_THREADS {
        let index = (current + distance) % MAX_THREADS;
        if (*threads.add(index)).state == ThreadState::Ready {
            return Some(index);
        }
    }
    None
}

unsafe fn stack_slice(index: usize) -> &'static mut [u8] {
    let stacks = addr_of_mut!(STACKS).cast::<ThreadStack>();
    let stack = stacks.add(index);
    unsafe { from_raw_parts_mut(addr_of_mut!((*stack).0).cast::<u8>(), STACK_SIZE) }
}

fn switch_current(next_state: ThreadState) -> bool {
    let interrupt_state = crate::arch::interrupts::disable();
    let current = unsafe { CURRENT_SLOT };
    let next = unsafe { find_ready(current) };
    let Some(next) = next else {
        crate::arch::interrupts::restore(interrupt_state);
        return false;
    };

    unsafe {
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        (*threads.add(current)).state = next_state;
        (*threads.add(next)).state = ThreadState::Running;
        CURRENT_SLOT = next;
        let old_stack = addr_of_mut!((*threads.add(current)).stack_pointer);
        let new_stack = (*threads.add(next)).stack_pointer;
        #[cfg(target_arch = "x86_64")]
        crate::arch::switch_thread(
            &mut *old_stack,
            new_stack,
            &mut (*threads.add(current)).fpu_context,
            &(*threads.add(next)).fpu_context,
        );
        #[cfg(target_arch = "aarch64")]
        crate::arch::switch_thread(
            &mut *old_stack,
            new_stack,
            &mut (*threads.add(current)).fpu_context,
            &(*threads.add(next)).fpu_context,
        );
    }
    crate::arch::interrupts::restore(interrupt_state);
    true
}

pub(crate) fn timer_tick() {
    let _ = switch_current(ThreadState::Ready);
}

pub(crate) fn current_thread_id() -> usize {
    let interrupt_state = crate::arch::interrupts::disable();
    let id = unsafe {
        if INITIALIZED {
            let threads = addr_of_mut!(THREADS).cast::<Thread>();
            (*threads.add(CURRENT_SLOT)).id
        } else {
            0
        }
    };
    crate::arch::interrupts::restore(interrupt_state);
    id
}

pub(crate) fn exit_current(result: *mut c_void) -> ! {
    let _interrupt_state = crate::arch::interrupts::disable();
    unsafe {
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        let current = CURRENT_SLOT;
        (*threads.add(current)).result = result;
        (*threads.add(current)).state = ThreadState::Exited;

        let joiner = (*threads.add(current)).joiner;
        (*threads.add(current)).joiner = 0;
        if joiner != 0 {
            if let Some(joiner_slot) = find_thread(joiner) {
                if (*threads.add(joiner_slot)).state == ThreadState::Blocked {
                    (*threads.add(joiner_slot)).state = ThreadState::Ready;
                }
            }
        }

        if let Some(next) = find_ready(current) {
            (*threads.add(next)).state = ThreadState::Running;
            CURRENT_SLOT = next;
            let old_stack = addr_of_mut!((*threads.add(current)).stack_pointer);
            let new_stack = (*threads.add(next)).stack_pointer;
            #[cfg(target_arch = "x86_64")]
            crate::arch::switch_thread(
                &mut *old_stack,
                new_stack,
                &mut (*threads.add(current)).fpu_context,
                &(*threads.add(next)).fpu_context,
            );
            #[cfg(target_arch = "aarch64")]
            crate::arch::switch_thread(
                &mut *old_stack,
                new_stack,
                &mut (*threads.add(current)).fpu_context,
                &(*threads.add(next)).fpu_context,
            );
        }
    }
    crate::arch::halt()
}

extern "C" fn thread_trampoline() -> ! {
    let interrupt_state = crate::arch::interrupts::disable();
    let current = unsafe { CURRENT_SLOT };
    let (start_routine, argument) = unsafe {
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        let thread = threads.add(current);
        ((*thread).start_routine, (*thread).argument)
    };
    crate::arch::interrupts::restore(interrupt_state);
    crate::arch::interrupts::enable();

    if let Some(start_routine) = start_routine {
        let result = unsafe { start_routine(argument) };
        exit_current(result);
    }
    exit_current(core::ptr::null_mut())
}
