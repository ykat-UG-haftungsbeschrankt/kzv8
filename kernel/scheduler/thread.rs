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
    fpu_context: crate::kernel::arch::FpuContext,
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
        fpu_context: crate::kernel::arch::FpuContext::INITIAL,
    };
}

#[derive(Clone, Copy)]
#[repr(align(16))]
pub(crate) struct ThreadStack([u8; Self::STACK_SIZE]);

impl ThreadStack {
    const STACK_SIZE: usize = 16 * 1024;
    /// # Safety
    /// `index` must be valid, and the caller must ensure exclusive access to that stack.
    pub(crate) unsafe fn slice_at(index: usize) -> &'static mut [u8] {
        let manager = ThreadManager::get_instance();
        let stacks = addr_of_mut!((*manager).stacks).cast::<ThreadStack>();
        let stack = stacks.wrapping_add(index);
        unsafe { from_raw_parts_mut(addr_of_mut!((*stack).0).cast::<u8>(), Self::STACK_SIZE) }
    }

    pub(crate) fn bounds(index: usize) -> (*mut c_void, usize) {
        if index == 0 {
            return <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::ThreadArchitecture>::
                current_thread_stack_bounds(Self::STACK_SIZE);
        }
        if index >= MAX_THREADS {
            return (core::ptr::null_mut(), 0);
        }

        let manager = ThreadManager::get_instance();
        let stacks = addr_of_mut!((*manager).stacks).cast::<ThreadStack>();
        (
            stacks.wrapping_add(index).cast::<c_void>(),
            Self::STACK_SIZE,
        )
    }
}

pub(crate) struct ThreadManager {
    threads: [Thread; Self::MAX_THREADS],
    tls_keys: [TlsKey; Self::MAX_TLS_KEYS],
    stacks: [ThreadStack; Self::MAX_THREADS],
    current_slot: usize,
    next_thread_id: usize,
    initialized: bool,
    fallback_errno: i32,
}

impl ThreadManager {
    const MAX_THREADS: usize = 8;
    const MAX_TLS_KEYS: usize = 64;
    const MAIN_THREAD_ID: usize = 1;

    const fn new() -> Self {
        Self {
            threads: [Thread::EMPTY; Self::MAX_THREADS],
            tls_keys: [TlsKey::EMPTY; Self::MAX_TLS_KEYS],
            stacks: [ThreadStack([0; ThreadStack::STACK_SIZE]); Self::MAX_THREADS],
            current_slot: 0,
            next_thread_id: MAIN_THREAD_ID + 1,
            initialized: false,
            fallback_errno: 0,
        }
    }

    fn get_instance() -> *mut Self {
        static mut INSTANCE: ThreadManager = ThreadManager::new();
        addr_of_mut!(INSTANCE)
    }

    pub(crate) fn initialize() {
        let interrupt_state = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::
            InterruptArchitecture>::disable_interrupts();
        let manager = unsafe { &mut *Self::get_instance() };
        manager.threads.fill(Thread::EMPTY);
        manager.tls_keys.fill(TlsKey::EMPTY);
        manager.threads[0] = Thread {
            id: MAIN_THREAD_ID,
            state: ThreadState::Running,
            ..Thread::EMPTY
        };
        manager.current_slot = 0;
        manager.next_thread_id = MAIN_THREAD_ID + 1;
        manager.initialized = true;
        <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::InterruptArchitecture>::
            restore_interrupts(interrupt_state);
    }

    fn errno_location() -> *mut i32 {
        let manager = Self::get_instance();
        unsafe {
            if (*manager).initialized {
                let current = (*manager).current_slot;
                let threads = addr_of_mut!((*manager).threads).cast::<Thread>();
                addr_of_mut!((*threads.add(current)).errno)
            } else {
                addr_of_mut!((*manager).fallback_errno)
            }
        }
    }

    unsafe fn find_thread(id: usize) -> Option<usize> {
        let manager = Self::get_instance();
        let threads = unsafe { addr_of_mut!((*manager).threads).cast::<Thread>() };
        for index in 0..Self::MAX_THREADS {
            let thread = unsafe { threads.add(index) };
            if unsafe { (*thread).state != ThreadState::Unused && (*thread).id == id } {
                return Some(index);
            }
        }
        None
    }

    unsafe fn find_ready(current: usize) -> Option<usize> {
        let manager = Self::get_instance();
        let threads = unsafe { addr_of_mut!((*manager).threads).cast::<Thread>() };
        for distance in 1..=Self::MAX_THREADS {
            let index = (current + distance) % Self::MAX_THREADS;
            if unsafe { (*threads.add(index)).state == ThreadState::Ready } {
                return Some(index);
            }
        }
        None
    }

    fn switch_current(next_state: ThreadState) -> bool {
        let interrupt_state = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::
            InterruptArchitecture>::disable_interrupts();
        let manager = Self::get_instance();
        let current = unsafe { (*manager).current_slot };
        let next = unsafe { Self::find_ready(current) };
        let Some(next) = next else {
            <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::InterruptArchitecture>::
                restore_interrupts(interrupt_state);
            return false;
        };

        unsafe {
            let threads = addr_of_mut!((*manager).threads).cast::<Thread>();
            (*threads.add(current)).state = next_state;
            (*threads.add(next)).state = ThreadState::Running;
            (*manager).current_slot = next;
            let old_stack = addr_of_mut!((*threads.add(current)).stack_pointer);
            let new_stack = (*threads.add(next)).stack_pointer;
            #[cfg(target_arch = "x86_64")]
            <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::ThreadArchitecture>::
                switch_thread(
                &mut *old_stack,
                new_stack,
                &mut (*threads.add(current)).fpu_context,
                &(*threads.add(next)).fpu_context,
            );
            #[cfg(target_arch = "aarch64")]
            <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::ThreadArchitecture>::
                switch_thread(
                &mut *old_stack,
                new_stack,
                &mut (*threads.add(current)).fpu_context,
                &(*threads.add(next)).fpu_context,
            );
        }
        <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::InterruptArchitecture>::
            restore_interrupts(interrupt_state);
        true
    }

    pub(crate) fn timer_tick() {
        let _ = Self::switch_current(ThreadState::Ready);
    }

    pub(crate) fn current_thread_id() -> usize {
        let interrupt_state = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::
            InterruptArchitecture>::disable_interrupts();
        let manager = Self::get_instance();
        let id = unsafe {
            if (*manager).initialized {
                let threads = addr_of_mut!((*manager).threads).cast::<Thread>();
                (*threads.add((*manager).current_slot)).id
            } else {
                0
            }
        };
        <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::InterruptArchitecture>::
            restore_interrupts(interrupt_state);
        id
    }

    pub(crate) fn exit_current(result: *mut c_void) -> ! {
        let _interrupt_state = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::
            InterruptArchitecture>::disable_interrupts();
        let manager = Self::get_instance();
        unsafe {
            let threads = addr_of_mut!((*manager).threads).cast::<Thread>();
            let current = (*manager).current_slot;
            (*threads.add(current)).result = result;
            (*threads.add(current)).state = ThreadState::Exited;

            let joiner = (*threads.add(current)).joiner;
            (*threads.add(current)).joiner = 0;
            if joiner != 0 {
                if let Some(joiner_slot) = Self::find_thread(joiner) {
                    if (*threads.add(joiner_slot)).state == ThreadState::Blocked {
                        (*threads.add(joiner_slot)).state = ThreadState::Ready;
                    }
                }
            }

            if let Some(next) = Self::find_ready(current) {
                (*threads.add(next)).state = ThreadState::Running;
                (*manager).current_slot = next;
                let old_stack = addr_of_mut!((*threads.add(current)).stack_pointer);
                let new_stack = (*threads.add(next)).stack_pointer;
                #[cfg(target_arch = "x86_64")]
                <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::ThreadArchitecture>::
                    switch_thread(
                    &mut *old_stack,
                    new_stack,
                    &mut (*threads.add(current)).fpu_context,
                    &(*threads.add(next)).fpu_context,
                );
                #[cfg(target_arch = "aarch64")]
                <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::ThreadArchitecture>::
                    switch_thread(
                    &mut *old_stack,
                    new_stack,
                    &mut (*threads.add(current)).fpu_context,
                    &(*threads.add(next)).fpu_context,
                );
            }
        }
        <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::CpuArchitecture>::halt()
    }

    extern "C" fn thread_trampoline() -> ! {
        let interrupt_state = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::
            InterruptArchitecture>::disable_interrupts();
        let manager = Self::get_instance();
        let current = unsafe { (*manager).current_slot };
        let (start_routine, argument) = unsafe {
            let threads = addr_of_mut!((*manager).threads).cast::<Thread>();
            let thread = threads.add(current);
            ((*thread).start_routine, (*thread).argument)
        };
        <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::InterruptArchitecture>::
            restore_interrupts(interrupt_state);
        <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::InterruptArchitecture>::
            enable_interrupts();

        if let Some(start_routine) = start_routine {
            let result = unsafe { start_routine(argument) };
            Self::exit_current(result);
        }
        Self::exit_current(core::ptr::null_mut())
    }
}
