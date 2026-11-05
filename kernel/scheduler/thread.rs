/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr::{addr_of_mut, NonNull};
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::kernel::arch::traits::{
    CpuArchitecture as _, InterruptArchitecture as _, ThreadArchitecture as _,
};
use crate::kernel::error::Error;
use crate::kernel::lib::hashmap::HashMap;
use crate::kernel::lib::list::{LinkedList, ListNode};

use super::sync::IrqSpinLock;

pub(crate) type StartRoutine = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
pub(crate) type FpuContext =
    <crate::kernel::arch::ThreadArchitecture as crate::kernel::arch::traits::ThreadArchitecture>::FpuContext;

#[repr(C)]
pub(crate) struct ThreadAttributes {
    pub(crate) stack_size: usize,
    pub(crate) detach_state: i32,
    pub(crate) stack_address: *mut c_void,
}

#[repr(C)]
pub(crate) struct PthreadMutex {
    pub(crate) owner: AtomicUsize,
}

#[repr(C)]
pub(crate) struct PthreadCondition {
    pub(crate) generation: AtomicUsize,
}

#[derive(Clone, Copy)]
pub(crate) struct TlsKey {
    pub(crate) active: bool,
    pub(crate) destructor: Option<unsafe extern "C" fn(*mut c_void)>,
}

impl TlsKey {
    pub(crate) const EMPTY: Self = Self {
        active: false,
        destructor: None,
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThreadState {
    Running,
    Ready,
    Blocked,
    Exited,
}

pub(crate) struct Thread {
    pub(crate) id: usize,
    pub(crate) stack_pointer: usize,
    pub(crate) state: ThreadState,
    pub(crate) start_routine: Option<StartRoutine>,
    pub(crate) argument: usize,
    pub(crate) result: usize,
    pub(crate) joiner: usize,
    pub(crate) detached: bool,
    pub(crate) errno: i32,
    pub(crate) tls: [usize; ThreadManager::MAX_TLS_KEYS],
    pub(crate) fpu_context: FpuContext,
    stack: Option<ThreadStack>,
    external_stack_address: usize,
    external_stack_size: usize,
}

impl Thread {
    fn main(stack_address: usize, stack_size: usize) -> Self {
        Self {
            id: 0,
            stack_pointer: 0,
            state: ThreadState::Running,
            start_routine: None,
            argument: 0,
            result: 0,
            joiner: 0,
            detached: false,
            errno: 0,
            tls: [0; ThreadManager::MAX_TLS_KEYS],
            fpu_context: FpuContext::INITIAL,
            stack: None,
            external_stack_address: stack_address,
            external_stack_size: stack_size,
        }
    }

    fn new(
        stack: ThreadStack,
        stack_pointer: usize,
        start_routine: StartRoutine,
        argument: *mut c_void,
        detached: bool,
    ) -> Self {
        Self {
            id: 0,
            stack_pointer,
            state: ThreadState::Ready,
            start_routine: Some(start_routine),
            argument: argument as usize,
            result: 0,
            joiner: 0,
            detached,
            errno: 0,
            tls: [0; ThreadManager::MAX_TLS_KEYS],
            fpu_context: FpuContext::INITIAL,
            stack: Some(stack),
            external_stack_address: 0,
            external_stack_size: 0,
        }
    }

    fn stack_bounds(&self) -> (*mut c_void, usize) {
        if let Some(stack) = &self.stack {
            stack.bounds()
        } else {
            (self.external_stack_address as *mut c_void, self.external_stack_size)
        }
    }
}

pub(crate) struct ThreadStack {
    bytes: Vec<u8>,
}

impl ThreadStack {
    pub(crate) const DEFAULT_SIZE: usize = 16 * 1024;
    pub(crate) const MIN_SIZE: usize = 16 * 1024;

    fn try_new(size: usize) -> Result<Self, Error> {
        if size < Self::MIN_SIZE {
            return Err(Error::Inval);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| Error::NoMem)?;
        bytes.resize(size, 0);
        Ok(Self { bytes })
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    fn bounds(&self) -> (*mut c_void, usize) {
        (self.bytes.as_ptr().cast_mut().cast(), self.bytes.len())
    }
}

struct SwitchContext {
    old_stack: *mut usize,
    new_stack: usize,
    old_fpu_context: *mut FpuContext,
    new_fpu_context: *const FpuContext,
}

pub(crate) struct ThreadManager {
    ready: LinkedList<Thread>,
    busy: LinkedList<Thread>,
    thread_nodes: HashMap<usize, usize>,
    pending_reap: Option<Box<ListNode<Thread>>>,
    tls_keys: [TlsKey; Self::MAX_TLS_KEYS],
    initialized: bool,
    fallback_errno: i32,
}

static THREAD_MANAGER: IrqSpinLock<ThreadManager> =
    IrqSpinLock::new(ThreadManager::new());

impl ThreadManager {
    pub(crate) const MAX_TLS_KEYS: usize = 64;
    pub(crate) const MAIN_THREAD_ID: usize = 1;

    const fn new() -> Self {
        Self {
            ready: LinkedList::new(),
            busy: LinkedList::new(),
            thread_nodes: HashMap::new(),
            pending_reap: None,
            tls_keys: [TlsKey::EMPTY; Self::MAX_TLS_KEYS],
            initialized: false,
            fallback_errno: 0,
        }
    }

    fn thread_node_by_id(&self, id: usize) -> Option<NonNull<ListNode<Thread>>> {
        NonNull::new((*self.thread_nodes.get(&id)?) as *mut ListNode<Thread>)
    }

    fn thread_ref(&self, node: NonNull<ListNode<Thread>>) -> &Thread {
        // SAFETY: callers pass nodes from the live thread registry.
        unsafe { &**node.as_ptr() }
    }

    fn thread_pointer(&self, node: NonNull<ListNode<Thread>>) -> NonNull<Thread> {
        NonNull::from(self.thread_ref(node))
    }

    fn thread_mut(&mut self, mut node: NonNull<ListNode<Thread>>) -> &mut Thread {
        // SAFETY: callers pass live registry nodes while holding exclusive manager access.
        unsafe { &mut **node.as_mut() }
    }

    fn thread_ref_from_pointer(&self, thread: NonNull<Thread>) -> &Thread {
        // SAFETY: callers resolve this pointer from the registry while holding the manager lock.
        unsafe { thread.as_ref() }
    }

    fn thread_mut_from_pointer(&mut self, mut thread: NonNull<Thread>) -> &mut Thread {
        // SAFETY: callers resolve this pointer from the registry while holding exclusive manager access.
        unsafe { thread.as_mut() }
    }

    fn remove_thread(&mut self, node: NonNull<ListNode<Thread>>) -> Option<Box<ListNode<Thread>>> {
        let (id, state) = {
            let thread = self.thread_ref(node);
            (thread.id, thread.state)
        };
        let removed = if state == ThreadState::Ready {
            // SAFETY: the registry points to this live node, and Ready threads are in ready.
            unsafe { ListNode::remove(&mut self.ready, node) }
        } else {
            // SAFETY: the registry points to this live node, and non-Ready threads are in busy.
            unsafe { ListNode::remove(&mut self.busy, node) }
        }?;
        self.thread_nodes.remove(&id);
        Some(removed)
    }

    fn wake_blocked(&mut self, node: NonNull<ListNode<Thread>>) -> bool {
        if self.thread_ref(node).state != ThreadState::Blocked {
            return false;
        }
        // SAFETY: the registry points to this live node, and Blocked threads are in busy.
        let removed = unsafe { ListNode::remove(&mut self.busy, node) };
        let Some(mut thread) = removed else {
            return false;
        };
        thread.state = ThreadState::Ready;
        self.ready.push_back(thread);
        true
    }

    pub(crate) fn initialize() -> bool {
        let (stack_address, stack_size) =
            crate::kernel::arch::ThreadArchitecture::current_thread_stack_bounds(
                ThreadStack::DEFAULT_SIZE,
            );
        let mut main_thread = Box::new(ListNode::new(Thread::main(
            stack_address as usize,
            stack_size,
        )));
        let id = addr_of_mut!(**main_thread) as usize;
        main_thread.id = id;
        let node_address = main_thread.as_mut() as *mut ListNode<Thread> as usize;
        let mut manager = THREAD_MANAGER.lock();
        manager.ready.clear();
        manager.busy.clear();
        manager.thread_nodes.clear();
        manager.pending_reap = None;
        manager.tls_keys.fill(TlsKey::EMPTY);
        manager.initialized = false;
        if !matches!(manager.thread_nodes.insert(id, node_address), Ok(None)) {
            return false;
        }
        manager.busy.push_back(main_thread);
        manager.initialized = true;
        true
    }

    pub(crate) fn errno_location() -> *mut i32 {
        let mut manager = THREAD_MANAGER.lock();
        if manager.initialized {
            if let Some(thread) = manager.busy.front_mut() {
                return addr_of_mut!(thread.errno);
            }
        }
        addr_of_mut!(manager.fallback_errno)
    }

    pub(crate) fn create_tls_key(
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> Option<u32> {
        let mut manager = THREAD_MANAGER.lock();
        for (index, key) in manager.tls_keys.iter_mut().enumerate() {
            if !key.active {
                *key = TlsKey {
                    active: true,
                    destructor,
                };
                return Some((index + 1) as u32);
            }
        }
        None
    }

    pub(crate) fn delete_tls_key(key: u32) -> bool {
        if key == 0 || key as usize > Self::MAX_TLS_KEYS {
            return false;
        }
        let mut manager = THREAD_MANAGER.lock();
        let index = key as usize - 1;
        if !manager.tls_keys[index].active {
            return false;
        }
        manager.tls_keys[index] = TlsKey::EMPTY;
        for thread in manager.ready.iter_mut() {
            thread.tls[index] = 0;
        }
        for thread in manager.busy.iter_mut() {
            thread.tls[index] = 0;
        }
        true
    }

    pub(crate) fn get_specific(key: u32) -> Option<*mut c_void> {
        if key == 0 || key as usize > Self::MAX_TLS_KEYS {
            return None;
        }
        let manager = THREAD_MANAGER.lock();
        let index = key as usize - 1;
        if !manager.initialized || !manager.tls_keys[index].active {
            return None;
        }
        manager
            .busy
            .front()
            .map(|thread| thread.tls[index] as *mut c_void)
    }

    pub(crate) fn set_specific(key: u32, value: *const c_void) -> bool {
        if key == 0 || key as usize > Self::MAX_TLS_KEYS {
            return false;
        }
        let mut manager = THREAD_MANAGER.lock();
        let index = key as usize - 1;
        if !manager.initialized || !manager.tls_keys[index].active {
            return false;
        }
        let Some(thread) = manager.busy.front_mut() else {
            return false;
        };
        thread.tls[index] = value as usize;
        true
    }

    pub(crate) fn create_thread(
        stack_size: usize,
        start_routine: StartRoutine,
        argument: *mut c_void,
        detached: bool,
    ) -> Result<usize, Error> {
        let mut stack = ThreadStack::try_new(stack_size)?;
        let stack_pointer = crate::kernel::arch::ThreadArchitecture::initialize_thread_stack(
            stack.as_mut_slice(),
            Self::thread_trampoline,
        );
        let mut thread = Box::new(ListNode::new(Thread::new(
            stack,
            stack_pointer,
            start_routine,
            argument,
            detached,
        )));
        let id = addr_of_mut!(**thread) as usize;
        thread.id = id;
        let node_address = thread.as_mut() as *mut ListNode<Thread> as usize;

        let mut manager = THREAD_MANAGER.lock();
        if !manager.initialized {
            return Err(Error::Again);
        }
        match manager.thread_nodes.insert(id, node_address) {
            Ok(None) => {}
            Ok(Some(previous)) => {
                let _ = manager.thread_nodes.insert(id, previous);
                return Err(Error::Again);
            }
            Err(_) => return Err(Error::NoMem),
        }
        manager.ready.push_back(thread);
        Ok(id)
    }

    pub(crate) fn switch_current(next_state: ThreadState) -> bool {
        let interrupt_state = crate::kernel::arch::InterruptArchitecture::disable_interrupts();
        if !matches!(next_state, ThreadState::Ready | ThreadState::Blocked) {
            crate::kernel::arch::InterruptArchitecture::restore_interrupts(interrupt_state);
            return false;
        }

        let context = {
            let mut manager = THREAD_MANAGER.lock();
            if !manager.initialized || manager.ready.is_empty() {
                None
            } else if let Some(mut current) = manager.busy.pop_front() {
                let old_stack = addr_of_mut!((*current).stack_pointer);
                let old_fpu_context = addr_of_mut!((*current).fpu_context);
                current.state = next_state;
                if next_state == ThreadState::Ready {
                    manager.ready.push_back(current);
                } else {
                    manager.busy.push_back(current);
                }

                if let Some(mut next) = manager.ready.pop_front() {
                    next.state = ThreadState::Running;
                    let context = SwitchContext {
                        old_stack,
                        new_stack: next.stack_pointer,
                        old_fpu_context,
                        new_fpu_context: addr_of_mut!((*next).fpu_context),
                    };
                    manager.busy.push_front(next);
                    Some(context)
                } else {
                    None
                }
            } else {
                None
            }
        };
        let Some(context) = context else {
            crate::kernel::arch::InterruptArchitecture::restore_interrupts(interrupt_state);
            return false;
        };

        // SAFETY: both thread boxes remain owned by the queues while their contexts are switched.
        unsafe { Self::switch_threads(context) };
        Self::reap_pending();
        crate::kernel::arch::InterruptArchitecture::restore_interrupts(interrupt_state);
        true
    }

    unsafe fn switch_threads(context: SwitchContext) {
        unsafe {
            crate::kernel::arch::ThreadArchitecture::switch_thread(
                &mut *context.old_stack,
                context.new_stack,
                &mut *context.old_fpu_context,
                &*context.new_fpu_context,
            );
        }
    }

    pub(crate) fn timer_tick() {
        let _ = Self::switch_current(ThreadState::Ready);
    }

    pub(crate) fn current_thread_id() -> usize {
        let manager = THREAD_MANAGER.lock();
        if manager.initialized {
            manager.busy.front().map_or(0, |thread| thread.id)
        } else {
            0
        }
    }

    pub(crate) fn contains_thread_by_id(id: usize) -> bool {
        THREAD_MANAGER.lock().thread_node_by_id(id).is_some()
    }

    fn thread_attributes(&self, thread: NonNull<Thread>) -> (*mut c_void, usize, bool) {
        let thread = self.thread_ref_from_pointer(thread);
        let (stack_address, stack_size) = thread.stack_bounds();
        (stack_address, stack_size, thread.detached)
    }

    pub(crate) fn thread_attributes_by_id(id: usize) -> Option<(*mut c_void, usize, bool)> {
        let manager = THREAD_MANAGER.lock();
        let node = manager.thread_node_by_id(id)?;
        Some(manager.thread_attributes(manager.thread_pointer(node)))
    }

    fn begin_join(
        &mut self,
        thread: NonNull<Thread>,
    ) -> Result<Option<usize>, Error> {
        let current_id = self.busy.front().map_or(0, |thread| thread.id);
        let (id, state, detached, joiner, result) = {
            let thread = self.thread_ref_from_pointer(thread);
            (
                thread.id,
                thread.state,
                thread.detached,
                thread.joiner,
                thread.result,
            )
        };
        if id == current_id {
            return Err(Error::Deadlk);
        }
        if detached || joiner != 0 {
            return Err(Error::Inval);
        }
        if state == ThreadState::Exited {
            Ok(Some(result))
        } else {
            self.thread_mut_from_pointer(thread).joiner = current_id;
            Ok(None)
        }
    }

    pub(crate) fn begin_join_by_id(id: usize) -> Result<Option<usize>, Error> {
        let (result, removed) = {
            let mut manager = THREAD_MANAGER.lock();
            if !manager.initialized {
                return Err(Error::Srch);
            }
            let Some(list_node) = manager.thread_node_by_id(id) else {
                return Err(Error::Srch);
            };
            let thread = manager.thread_pointer(list_node);
            let result = manager.begin_join(thread)?;
            let removed = if result.is_some() {
                Some(
                    manager
                        .remove_thread(list_node)
                        .ok_or(Error::Srch)?,
                )
            } else {
                None
            };
            (result, removed)
        };
        drop(removed);
        Ok(result)
    }

    fn cancel_join(&mut self, thread: NonNull<Thread>) {
        let current_id = self.busy.front().map_or(0, |thread| thread.id);
        let thread = self.thread_mut_from_pointer(thread);
        if thread.joiner == current_id {
            thread.joiner = 0;
        }
    }

    pub(crate) fn cancel_join_by_id(id: usize) {
        let mut manager = THREAD_MANAGER.lock();
        let Some(node) = manager.thread_node_by_id(id) else {
            return;
        };
        let thread = manager.thread_pointer(node);
        manager.cancel_join(thread);
    }

    fn finish_join(&self, thread: NonNull<Thread>) -> Result<usize, Error> {
        let current_id = self.busy.front().map_or(0, |thread| thread.id);
        let thread = self.thread_ref_from_pointer(thread);
        if thread.state != ThreadState::Exited {
            return Err(Error::Deadlk);
        }
        if thread.joiner != current_id {
            return Err(Error::Inval);
        }
        Ok(thread.result)
    }

    pub(crate) fn finish_join_by_id(id: usize) -> Result<usize, Error> {
        let (result, removed) = {
            let mut manager = THREAD_MANAGER.lock();
            let Some(node) = manager.thread_node_by_id(id) else {
                return Err(Error::Srch);
            };
            let thread = manager.thread_pointer(node);
            let result = manager.finish_join(thread)?;
            let removed = manager
                .remove_thread(node)
                .ok_or(Error::Srch)?;
            (result, removed)
        };
        drop(removed);
        Ok(result)
    }

    fn detach(&mut self, thread: NonNull<Thread>) -> Result<bool, Error> {
        let (state, detached, joiner) = {
            let thread = self.thread_ref_from_pointer(thread);
            (thread.state, thread.detached, thread.joiner)
        };
        if detached || joiner != 0 {
            return Err(Error::Inval);
        }
        if state == ThreadState::Exited {
            Ok(true)
        } else {
            self.thread_mut_from_pointer(thread).detached = true;
            Ok(false)
        }
    }

    pub(crate) fn detach_by_id(id: usize) -> Result<(), Error> {
        let removed = {
            let mut manager = THREAD_MANAGER.lock();
            let Some(node) = manager.thread_node_by_id(id) else {
                return Err(Error::Srch);
            };
            let thread = manager.thread_pointer(node);
            if manager.detach(thread)? {
                Some(
                    manager
                        .remove_thread(node)
                        .ok_or(Error::Srch)?,
                )
            } else {
                None
            }
        };
        drop(removed);
        Ok(())
    }

    fn reap_pending() {
        let pending = THREAD_MANAGER.lock().pending_reap.take();
        drop(pending);
    }

    pub(crate) fn exit_current(result: *mut c_void) -> ! {
        let _interrupt_state =
            crate::kernel::arch::InterruptArchitecture::disable_interrupts();
        let context = {
            let mut manager = THREAD_MANAGER.lock();
            if let Some(mut current) = manager.busy.pop_front() {
                current.result = result as usize;
                current.state = ThreadState::Exited;
                if current.joiner != 0 {
                    if let Some(joiner) = manager.thread_node_by_id(current.joiner) {
                        manager.wake_blocked(joiner);
                    }
                }
                let old_stack = addr_of_mut!((*current).stack_pointer);
                let old_fpu_context = addr_of_mut!((*current).fpu_context);
                if current.detached && manager.pending_reap.is_none() {
                    manager.thread_nodes.remove(&current.id);
                    manager.pending_reap = Some(current);
                } else {
                    manager.busy.push_back(current);
                }

                if let Some(mut next) = manager.ready.pop_front() {
                    next.state = ThreadState::Running;
                    let context = SwitchContext {
                        old_stack,
                        new_stack: next.stack_pointer,
                        old_fpu_context,
                        new_fpu_context: addr_of_mut!((*next).fpu_context),
                    };
                    manager.busy.push_front(next);
                    Some(context)
                } else {
                    None
                }
            } else {
                None
            }
        };
        let Some(context) = context else {
            crate::kernel::arch::CpuArchitecture::halt();
        };
        // SAFETY: the exiting node is retained until another thread has switched onto its stack.
        unsafe { Self::switch_threads(context) };
        crate::kernel::arch::CpuArchitecture::halt()
    }

    pub(crate) extern "C" fn thread_trampoline() -> ! {
        let interrupt_state = crate::kernel::arch::InterruptArchitecture::disable_interrupts();
        Self::reap_pending();
        let start = {
            let manager = THREAD_MANAGER.lock();
            manager.busy.front().and_then(|thread| {
                thread
                    .start_routine
                    .map(|start_routine| (start_routine, thread.argument as *mut c_void))
            })
        };
        let Some((start_routine, argument)) = start else {
            crate::kernel::arch::CpuArchitecture::halt();
        };
        crate::kernel::arch::InterruptArchitecture::restore_interrupts(interrupt_state);
        crate::kernel::arch::InterruptArchitecture::enable_interrupts();

        let result = unsafe { start_routine(argument) };
        Self::exit_current(result)
    }
}

#[no_mangle]
pub extern "C" fn __errno_location() -> *mut i32 {
    ThreadManager::errno_location()
}
