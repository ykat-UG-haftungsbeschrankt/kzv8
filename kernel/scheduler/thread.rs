/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr::{addr_of_mut, NonNull};
use core::sync::atomic::AtomicUsize;

use crate::kernel::arch::traits::{
    CpuArchitecture as _, InterruptArchitecture as _, MemoryArchitecture as _,
    ThreadArchitecture as _,
};
use crate::kernel::error::Error;
use crate::kernel::lib::hashmap::HashMap;
use crate::kernel::lib::list::{LinkedList, ListNode};
use crate::kernel::process::Process;
use crate::kernel::namespace::NamespaceSet;

use super::sync::IrqSpinLock;

#[path = "thread/kernel.rs"]
pub(crate) mod kernel;
#[path = "thread/userspace.rs"]
pub(crate) mod userspace;
#[path = "thread/vm.rs"]
pub(crate) mod vm;
#[path = "thread/smoke.rs"]
pub(crate) mod smoke;

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
    #[allow(dead_code)]
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

pub(crate) enum ExecutionContext {
    Kernel(kernel::Context),
    Userspace(userspace::Context),
    #[cfg(feature = "virtualization")]
    Vm(vm::VcpuContext),
}

impl ExecutionContext {
    fn user_kernel_stack_pointer(&self) -> Option<usize> {
        match self {
            Self::Userspace(context) => Some(context.kernel_stack_pointer()),
            Self::Kernel(_) => None,
            #[cfg(feature = "virtualization")]
            Self::Vm(_) => None,
        }
    }

    fn kernel(&self) -> &kernel::Context {
        match self {
            Self::Kernel(context) => context,
            Self::Userspace(context) => context.kernel(),
            #[cfg(feature = "virtualization")]
            Self::Vm(context) => context.kernel(),
        }
    }

    fn kernel_mut(&mut self) -> &mut kernel::Context {
        match self {
            Self::Kernel(context) => context,
            Self::Userspace(context) => context.kernel_mut(),
            #[cfg(feature = "virtualization")]
            Self::Vm(context) => context.kernel_mut(),
        }
    }
}

pub(crate) struct Thread {
    // Thread IDs intentionally use the stable address of the Thread inside its Box.
    pub(crate) id: usize,
    process: Arc<Process>,
    pub(crate) context: ExecutionContext,
    pub(crate) state: ThreadState,
    pub(crate) start_routine: Option<StartRoutine>,
    pub(crate) argument: usize,
    pub(crate) result: usize,
    pub(crate) joiner: usize,
    pub(crate) detached: bool,
    pub(crate) errno: i32,
    pub(crate) tls: [usize; ThreadManager::MAX_TLS_KEYS],
    stack: Option<ThreadStack>,
    external_stack_address: usize,
    external_stack_size: usize,
}

impl Thread {
    fn main(stack_address: usize, stack_size: usize, process: Arc<Process>) -> Self {
        Self {
            id: 0,
            process,
            context: ExecutionContext::Kernel(kernel::Context::main()),
            state: ThreadState::Running,
            start_routine: None,
            argument: 0,
            result: 0,
            joiner: 0,
            detached: false,
            errno: 0,
            tls: [0; ThreadManager::MAX_TLS_KEYS],
            stack: None,
            external_stack_address: stack_address,
            external_stack_size: stack_size,
        }
    }

    fn new(
        stack: ThreadStack,
        context: kernel::Context,
        process: Arc<Process>,
        start_routine: StartRoutine,
        argument: *mut c_void,
        detached: bool,
    ) -> Self {
        Self {
            id: 0,
            process,
            context: ExecutionContext::Kernel(context),
            state: ThreadState::Ready,
            start_routine: Some(start_routine),
            argument: argument as usize,
            result: 0,
            joiner: 0,
            detached,
            errno: 0,
            tls: [0; ThreadManager::MAX_TLS_KEYS],
            stack: Some(stack),
            external_stack_address: 0,
            external_stack_size: 0,
        }
    }

    fn new_userspace(
        stack: ThreadStack,
        context: userspace::Context,
        process: Arc<Process>,
        detached: bool,
    ) -> Self {
        Self {
            id: 0,
            process,
            context: ExecutionContext::Userspace(context),
            state: ThreadState::Ready,
            start_routine: None,
            argument: 0,
            result: 0,
            joiner: 0,
            detached,
            errno: 0,
            tls: [0; ThreadManager::MAX_TLS_KEYS],
            stack: Some(stack),
            external_stack_address: 0,
            external_stack_size: 0,
        }
    }

    #[cfg(feature = "virtualization")]
    fn new_vm(
        stack: ThreadStack,
        context: vm::VcpuContext,
        process: Arc<Process>,
        detached: bool,
    ) -> Self {
        Self {
            id: 0,
            process,
            context: ExecutionContext::Vm(context),
            state: ThreadState::Ready,
            start_routine: None,
            argument: 0,
            result: 0,
            joiner: 0,
            detached,
            errno: 0,
            tls: [0; ThreadManager::MAX_TLS_KEYS],
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
    old_context: *mut kernel::Context,
    new_context: *const kernel::Context,
    new_user_kernel_stack: Option<usize>,
    new_address_space_root: usize,
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
    #[allow(dead_code)]
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

    #[allow(dead_code)]
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
        let Ok(process) = Process::new() else {
            return false;
        };
        let process_root = process.address_space().root();
        let (stack_address, stack_size) =
            crate::kernel::arch::ThreadArchitecture::current_thread_stack_bounds(
                ThreadStack::DEFAULT_SIZE,
            );
        let mut main_thread = Box::new(ListNode::new(Thread::main(
            stack_address as usize,
            stack_size,
            process,
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
        if !crate::kernel::arch::MemoryArchitecture::activate_address_space(process_root) {
            crate::kernel::arch::CpuArchitecture::halt();
        }
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
        let process = Self::current_process()?;
        Self::create_thread_in_process(process, stack_size, start_routine, argument, detached)
    }

    pub(crate) fn create_thread_in_process(
        process: Arc<Process>,
        stack_size: usize,
        start_routine: StartRoutine,
        argument: *mut c_void,
        detached: bool,
    ) -> Result<usize, Error> {
        let mut stack = ThreadStack::try_new(stack_size)?;
        let context = kernel::Context::load(stack.as_mut_slice(), Self::thread_trampoline);
        let mut thread = Box::new(ListNode::new(Thread::new(
            stack,
            context,
            process,
            start_routine,
            argument,
            detached,
        )));
        Self::enqueue_thread(thread)
    }

    pub(crate) fn create_userspace_thread(
        stack_size: usize,
        entry: usize,
        user_stack_pointer: usize,
        detached: bool,
    ) -> Result<usize, Error> {
        let process = Self::current_process()?;
        Self::create_userspace_thread_in_process(
            process,
            stack_size,
            entry,
            user_stack_pointer,
            detached,
        )
    }

    pub(crate) fn create_process() -> Result<Arc<Process>, Error> {
        let parent = Self::current_process()?;
        Process::new_in_namespace_set_with_credentials(
            parent.namespace_set(),
            parent.credentials()?,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn create_container_process() -> Result<Arc<Process>, Error> {
        let parent = Self::current_process()?;
        Process::new_in_namespace_set_with_credentials(
            NamespaceSet::new_container(),
            parent.credentials()?,
        )
    }

    pub(crate) fn create_userspace_thread_in_process(
        process: Arc<Process>,
        stack_size: usize,
        entry: usize,
        user_stack_pointer: usize,
        detached: bool,
    ) -> Result<usize, Error> {
        let mut stack = ThreadStack::try_new(stack_size)?;
        let address_space = process.address_space();
        let context = userspace::Context::load(
            stack.as_mut_slice(),
            Self::thread_trampoline,
            address_space,
            entry,
            user_stack_pointer,
        )?;
        let thread = Box::new(ListNode::new(Thread::new_userspace(
            stack, context, process, detached,
        )));
        Self::enqueue_thread(thread)
    }

    #[cfg(feature = "virtualization")]
    pub(crate) fn create_vm_thread(
        stack_size: usize,
        image: &[u8],
        detached: bool,
    ) -> Result<usize, crate::kernel::arch::VirtualizationError> {
        let process = Self::current_process().map_err(|error| match error {
            Error::NoMem => crate::kernel::arch::VirtualizationError::ResourceExhausted,
            _ => crate::kernel::arch::VirtualizationError::HostSetupFailed,
        })?;
        let mut stack = ThreadStack::try_new(stack_size).map_err(|error| match error {
            Error::NoMem => crate::kernel::arch::VirtualizationError::ResourceExhausted,
            _ => crate::kernel::arch::VirtualizationError::InvalidGuestImage,
        })?;
        let context = vm::VcpuContext::load(
            stack.as_mut_slice(),
            Self::thread_trampoline,
            image,
        )?;
        let thread = Box::new(ListNode::new(Thread::new_vm(
            stack, context, process, detached,
        )));
        Self::enqueue_thread(thread).map_err(|error| match error {
            Error::NoMem => crate::kernel::arch::VirtualizationError::ResourceExhausted,
            _ => crate::kernel::arch::VirtualizationError::HostSetupFailed,
        })
    }

    #[cfg(feature = "virtualization")]
    pub(crate) fn create_vm_thread_with_pci_function(
        stack_size: usize,
        image: &[u8],
        function: crate::kernel::vm::device::passthru::PciFunction,
        detached: bool,
    ) -> Result<usize, crate::kernel::arch::VirtualizationError> {
        let process = Self::current_process().map_err(|error| match error {
            Error::NoMem => crate::kernel::arch::VirtualizationError::ResourceExhausted,
            _ => crate::kernel::arch::VirtualizationError::HostSetupFailed,
        })?;
        let mut stack = ThreadStack::try_new(stack_size).map_err(|error| match error {
            Error::NoMem => crate::kernel::arch::VirtualizationError::ResourceExhausted,
            _ => crate::kernel::arch::VirtualizationError::InvalidGuestImage,
        })?;
        let context = vm::VcpuContext::load_with_pci_function(
            stack.as_mut_slice(),
            Self::thread_trampoline,
            image,
            function,
        )?;
        let thread = Box::new(ListNode::new(Thread::new_vm(
            stack, context, process, detached,
        )));
        Self::enqueue_thread(thread).map_err(|error| match error {
            Error::NoMem => crate::kernel::arch::VirtualizationError::ResourceExhausted,
            _ => crate::kernel::arch::VirtualizationError::HostSetupFailed,
        })
    }

    fn enqueue_thread(mut thread: Box<ListNode<Thread>>) -> Result<usize, Error> {
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
                let old_context = current.context.kernel_mut() as *mut kernel::Context;
                current.state = next_state;
                if next_state == ThreadState::Ready {
                    manager.ready.push_back(current);
                } else {
                    manager.busy.push_back(current);
                }

                if let Some(mut next) = manager.ready.pop_front() {
                    next.state = ThreadState::Running;
                    let context = SwitchContext {
                        old_context,
                        new_context: next.context.kernel() as *const kernel::Context,
                        new_user_kernel_stack: next.context.user_kernel_stack_pointer(),
                        new_address_space_root: next.process.address_space().root(),
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

        if let Some(kernel_stack_pointer) = context.new_user_kernel_stack {
            if !crate::kernel::arch::ThreadArchitecture::set_user_kernel_stack(
                kernel_stack_pointer,
            ) {
                crate::kernel::arch::CpuArchitecture::halt();
            }
        }
        if !crate::kernel::arch::MemoryArchitecture::activate_address_space(
            context.new_address_space_root,
        ) {
            crate::kernel::arch::CpuArchitecture::halt();
        }

        // SAFETY: both thread boxes remain owned by the queues while their contexts are switched.
        unsafe { Self::switch_threads(context) };
        Self::reap_pending();
        crate::kernel::arch::InterruptArchitecture::restore_interrupts(interrupt_state);
        true
    }

    unsafe fn switch_threads(context: SwitchContext) {
        unsafe {
            (&mut *context.old_context).restore(&*context.new_context);
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

    pub(crate) fn current_process() -> Result<Arc<Process>, Error> {
        let manager = THREAD_MANAGER.lock();
        if !manager.initialized {
            return Err(Error::Again);
        }
        manager
            .busy
            .front()
            .map(|thread| Arc::clone(&thread.process))
            .ok_or(Error::Again)
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
                let old_context = current.context.kernel_mut() as *mut kernel::Context;
                if current.detached && manager.pending_reap.is_none() {
                    manager.thread_nodes.remove(&current.id);
                    manager.pending_reap = Some(current);
                } else {
                    manager.busy.push_back(current);
                }

                if let Some(mut next) = manager.ready.pop_front() {
                    next.state = ThreadState::Running;
                    let context = SwitchContext {
                        old_context,
                        new_context: next.context.kernel() as *const kernel::Context,
                        new_user_kernel_stack: next.context.user_kernel_stack_pointer(),
                        new_address_space_root: next.process.address_space().root(),
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
        if let Some(kernel_stack_pointer) = context.new_user_kernel_stack {
            if !crate::kernel::arch::ThreadArchitecture::set_user_kernel_stack(
                kernel_stack_pointer,
            ) {
                crate::kernel::arch::CpuArchitecture::halt();
            }
        }
        if !crate::kernel::arch::MemoryArchitecture::activate_address_space(
            context.new_address_space_root,
        ) {
            crate::kernel::arch::CpuArchitecture::halt();
        }
        // SAFETY: the exiting node is retained until another thread has switched onto its stack.
        unsafe { Self::switch_threads(context) };
        crate::kernel::arch::CpuArchitecture::halt()
    }

    pub(crate) extern "C" fn thread_trampoline() -> ! {
        let interrupt_state = crate::kernel::arch::InterruptArchitecture::disable_interrupts();
        Self::reap_pending();
        #[derive(Clone, Copy)]
        enum Start {
            Kernel(StartRoutine, *mut c_void),
            Userspace(NonNull<userspace::Context>),
            #[cfg(feature = "virtualization")]
            Vm(NonNull<vm::VcpuContext>),
        }
        let start = {
            let manager = THREAD_MANAGER.lock();
            manager.busy.front().and_then(|thread| match &thread.context {
                ExecutionContext::Kernel(_) => thread.start_routine.map(|start_routine| {
                    Start::Kernel(start_routine, thread.argument as *mut c_void)
                }),
                ExecutionContext::Userspace(context) => {
                    Some(Start::Userspace(NonNull::from(context)))
                }
                #[cfg(feature = "virtualization")]
                ExecutionContext::Vm(context) => Some(Start::Vm(NonNull::from(context))),
            })
        };
        let Some(start) = start else {
            crate::kernel::arch::CpuArchitecture::halt();
        };
        if let Start::Userspace(context) = start {
            // SAFETY: the running thread stays owned by the busy queue while it enters userspace.
            unsafe { context.as_ref().restore() };
        }
        #[cfg(feature = "virtualization")]
        if let Start::Vm(context) = start {
            // SAFETY: the running thread owns its vCPU context until it exits.
            crate::kernel::arch::InterruptArchitecture::restore_interrupts(interrupt_state);
            crate::kernel::arch::InterruptArchitecture::enable_interrupts();
            unsafe { vm::VcpuContext::run(context) };
        }
        let Start::Kernel(start_routine, argument) = start else {
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
