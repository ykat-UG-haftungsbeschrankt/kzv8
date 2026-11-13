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
    ThreadArchitecture as _, TimerArchitecture as _,
};
use crate::kernel::error::Error;
use crate::kernel::lib::hashmap::HashMap;
use crate::kernel::lib::list::{LinkedList, ListNode};
use crate::kernel::process::{
    Process, ResourceUsageSnapshot, SchedulingParameters, SporadicParameters,
};
use crate::kernel::namespace::NamespaceSet;
use crate::kernel::signal::{SignalInfo, SignalQueue, SignalSet};

use super::sync::IrqSpinLock;

const SCHED_FIFO_POLICY: i32 = 1;
const SCHED_RR_POLICY: i32 = 2;
const SCHED_SPORADIC_POLICY: i32 = 3;
const SCHEDULER_TICK_NS: u64 = 10_000_000;
pub(crate) const MAX_SPORADIC_REPLENISHMENTS: u32 = 64;

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
    pub(crate) inherit_schedule: i32,
    pub(crate) scheduler_policy: i32,
    pub(crate) scheduler_priority: i32,
    pub(crate) sporadic_low_priority: i32,
    pub(crate) sporadic_period_ns: u64,
    pub(crate) sporadic_budget_ns: u64,
    pub(crate) sporadic_max_replenishments: u32,
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

#[derive(Default)]
struct ThreadResourceUsage {
    user_cpu_time_ns: u64,
    system_cpu_time_ns: u64,
}

#[derive(Clone, Copy, Default)]
struct Replenishment {
    deadline_ns: u64,
    budget_ns: u64,
}

#[derive(Clone, Copy)]
struct SporadicServer {
    parameters: Option<SporadicParameters>,
    remaining_budget_ns: u64,
    replenishments: [Replenishment; MAX_SPORADIC_REPLENISHMENTS as usize],
    replenishment_count: usize,
}

impl Default for SporadicServer {
    fn default() -> Self {
        Self {
            parameters: None,
            remaining_budget_ns: 0,
            replenishments: [Replenishment::default(); MAX_SPORADIC_REPLENISHMENTS as usize],
            replenishment_count: 0,
        }
    }
}

impl SporadicServer {
    fn configured(scheduling: SchedulingParameters) -> Self {
        let mut server = Self::default();
        server.configure(scheduling);
        server
    }

    fn configure(&mut self, scheduling: SchedulingParameters) {
        *self = Self::default();
        if scheduling.policy == SCHED_SPORADIC_POLICY {
            if let Some(parameters) = scheduling.sporadic {
                self.remaining_budget_ns = parameters.initial_budget_ns;
                self.parameters = Some(parameters);
            }
        }
    }

    fn replenish_due(&mut self, now_ns: u64) {
        let Some(parameters) = self.parameters else {
            return;
        };
        let mut index = 0;
        while index < self.replenishment_count {
            let replenishment = self.replenishments[index];
            if replenishment.deadline_ns <= now_ns {
                self.remaining_budget_ns = self
                    .remaining_budget_ns
                    .saturating_add(replenishment.budget_ns)
                    .min(parameters.initial_budget_ns);
                self.replenishment_count -= 1;
                self.replenishments[index] = self.replenishments[self.replenishment_count];
                self.replenishments[self.replenishment_count] = Replenishment::default();
            } else {
                index += 1;
            }
        }
    }

    fn charge(&mut self, elapsed_ns: u64, now_ns: u64) {
        let Some(parameters) = self.parameters else {
            return;
        };
        self.replenish_due(now_ns);
        let charged_ns = elapsed_ns.min(self.remaining_budget_ns);
        if charged_ns == 0 {
            return;
        }
        self.remaining_budget_ns -= charged_ns;
        let unrounded_deadline = now_ns.saturating_add(parameters.replenishment_period_ns);
        let rounded_deadline = unrounded_deadline
            .checked_add(SCHEDULER_TICK_NS - 1)
            .map(|deadline| deadline / SCHEDULER_TICK_NS * SCHEDULER_TICK_NS)
            .unwrap_or(u64::MAX);
        if let Some(replenishment) = self.replenishments[..self.replenishment_count]
            .iter_mut()
            .find(|replenishment| replenishment.deadline_ns == rounded_deadline)
        {
            replenishment.budget_ns = replenishment.budget_ns.saturating_add(charged_ns);
            return;
        }
        let maximum = (parameters.max_replenishments as usize)
            .min(MAX_SPORADIC_REPLENISHMENTS as usize);
        if self.replenishment_count < maximum {
            self.replenishments[self.replenishment_count] = Replenishment {
                deadline_ns: rounded_deadline,
                budget_ns: charged_ns,
            };
            self.replenishment_count += 1;
        } else if maximum != 0 {
            let latest = self.replenishments[..self.replenishment_count]
                .iter_mut()
                .max_by_key(|replenishment| replenishment.deadline_ns)
                .expect("nonzero sporadic replenishment count");
            latest.budget_ns = latest.budget_ns.saturating_add(charged_ns);
            latest.deadline_ns = latest.deadline_ns.max(rounded_deadline);
        }
    }
}

impl ThreadResourceUsage {
    fn total_cpu_time_ns(&self) -> u64 {
        self.user_cpu_time_ns
            .saturating_add(self.system_cpu_time_ns)
    }
}

#[cfg(test)]
mod sporadic_tests {
    use super::{SchedulingParameters, SporadicParameters, SporadicServer, SCHED_SPORADIC_POLICY};

    fn configuration(max_replenishments: u32) -> SchedulingParameters {
        SchedulingParameters {
            policy: SCHED_SPORADIC_POLICY,
            priority: 80,
            sporadic: Some(SporadicParameters {
                low_priority: 20,
                replenishment_period_ns: 100_000_000,
                initial_budget_ns: 10_000_000,
                max_replenishments,
            }),
        }
    }

    #[test]
    fn replenishment_is_rounded_up_to_scheduler_tick() {
        let mut server = SporadicServer::configured(configuration(4));
        server.charge(10_000_000, 10_000_001);

        assert_eq!(server.remaining_budget_ns, 0);
        assert_eq!(server.replenishment_count, 1);
        assert_eq!(server.replenishments[0].deadline_ns, 120_000_000);

        server.replenish_due(119_999_999);
        assert_eq!(server.remaining_budget_ns, 0);
        server.replenish_due(120_000_000);
        assert_eq!(server.remaining_budget_ns, 10_000_000);
    }

    #[test]
    fn replenishments_coalesce_at_configured_maximum() {
        let mut server = SporadicServer::configured(configuration(1));
        server.charge(1_000_000, 10_000_000);
        server.charge(1_000_000, 20_000_000);

        assert_eq!(server.replenishment_count, 1);
        assert_eq!(server.replenishments[0].deadline_ns, 120_000_000);
        assert_eq!(server.replenishments[0].budget_ns, 2_000_000);
        server.replenish_due(119_999_999);
        assert_eq!(server.remaining_budget_ns, 8_000_000);
        server.replenish_due(120_000_000);
        assert_eq!(server.remaining_budget_ns, 10_000_000);
    }
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
    scheduler_override: Option<SchedulingParameters>,
    sporadic_server: SporadicServer,
    pub(crate) context: ExecutionContext,
    pub(crate) state: ThreadState,
    pub(crate) start_routine: Option<StartRoutine>,
    pub(crate) argument: usize,
    pub(crate) result: usize,
    pub(crate) joiner: usize,
    pub(crate) detached: bool,
    pub(crate) errno: i32,
    resource_usage: ThreadResourceUsage,
    in_kernel: bool,
    running_since_ns: Option<u64>,
    cleanup_handlers: Vec<CleanupHandler>,
    pub(crate) tls: [usize; ThreadManager::MAX_TLS_KEYS],
    signal_mask: SignalSet,
    pending_signals: SignalQueue,
    signal_wait_set: Option<SignalSet>,
    signal_wake_pending: bool,
    cancellation_requested: bool,
    cancellation_enabled: bool,
    sleep_deadline_ns: Option<u64>,
    process_stop_wait: bool,
    process_stop_wake: bool,
    stack: Option<ThreadStack>,
    external_stack_address: usize,
    external_stack_size: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct CleanupHandler {
    pub(crate) routine: unsafe extern "C" fn(*mut c_void),
    pub(crate) argument: usize,
}

impl Thread {
    fn scheduling_configuration(&self) -> SchedulingParameters {
        self.scheduler_override
            .unwrap_or_else(|| self.process.scheduling_configuration())
    }

    fn scheduler_parameters(&self) -> (i32, i32) {
        let scheduling = self.scheduling_configuration();
        (scheduling.policy, scheduling.priority)
    }

    fn effective_priority(&self, scheduling: SchedulingParameters) -> i32 {
        if scheduling.policy == SCHED_SPORADIC_POLICY
            && self.sporadic_server.remaining_budget_ns == 0
        {
            scheduling
                .sporadic
                .map_or(scheduling.priority, |parameters| parameters.low_priority)
        } else {
            scheduling.priority
        }
    }

    fn scheduling_rank(&self) -> i32 {
        let scheduling = self.scheduling_configuration();
        if matches!(
            scheduling.policy,
            SCHED_FIFO_POLICY | SCHED_RR_POLICY | SCHED_SPORADIC_POLICY
        ) {
            100 + self.effective_priority(scheduling)
        } else {
            20 - self.process.nice_priority()
        }
    }

    fn main(stack_address: usize, stack_size: usize, process: Arc<Process>) -> Self {
        let sporadic_server = SporadicServer::configured(process.scheduling_configuration());
        Self {
            id: 0,
            process,
            scheduler_override: None,
            sporadic_server,
            context: ExecutionContext::Kernel(kernel::Context::main()),
            state: ThreadState::Running,
            start_routine: None,
            argument: 0,
            result: 0,
            joiner: 0,
            detached: false,
            errno: 0,
            resource_usage: ThreadResourceUsage::default(),
            in_kernel: false,
            running_since_ns: None,
            cleanup_handlers: Vec::new(),
            tls: [0; ThreadManager::MAX_TLS_KEYS],
            signal_mask: SignalSet::EMPTY,
            pending_signals: SignalQueue::new(),
            signal_wait_set: None,
            signal_wake_pending: false,
            cancellation_requested: false,
            cancellation_enabled: true,
            sleep_deadline_ns: None,
            process_stop_wait: false,
            process_stop_wake: false,
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
        let sporadic_server = SporadicServer::configured(process.scheduling_configuration());
        Self {
            id: 0,
            process,
            scheduler_override: None,
            sporadic_server,
            context: ExecutionContext::Kernel(context),
            state: ThreadState::Ready,
            start_routine: Some(start_routine),
            argument: argument as usize,
            result: 0,
            joiner: 0,
            detached,
            errno: 0,
            resource_usage: ThreadResourceUsage::default(),
            in_kernel: false,
            running_since_ns: None,
            cleanup_handlers: Vec::new(),
            tls: [0; ThreadManager::MAX_TLS_KEYS],
            signal_mask: SignalSet::EMPTY,
            pending_signals: SignalQueue::new(),
            signal_wait_set: None,
            signal_wake_pending: false,
            cancellation_requested: false,
            cancellation_enabled: true,
            sleep_deadline_ns: None,
            process_stop_wait: false,
            process_stop_wake: false,
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
        let sporadic_server = SporadicServer::configured(process.scheduling_configuration());
        Self {
            id: 0,
            process,
            scheduler_override: None,
            sporadic_server,
            context: ExecutionContext::Userspace(context),
            state: ThreadState::Ready,
            start_routine: None,
            argument: 0,
            result: 0,
            joiner: 0,
            detached,
            errno: 0,
            resource_usage: ThreadResourceUsage::default(),
            in_kernel: false,
            running_since_ns: None,
            cleanup_handlers: Vec::new(),
            tls: [0; ThreadManager::MAX_TLS_KEYS],
            signal_mask: SignalSet::EMPTY,
            pending_signals: SignalQueue::new(),
            signal_wait_set: None,
            signal_wake_pending: false,
            cancellation_requested: false,
            cancellation_enabled: true,
            sleep_deadline_ns: None,
            process_stop_wait: false,
            process_stop_wake: false,
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
        let sporadic_server = SporadicServer::configured(process.scheduling_configuration());
        Self {
            id: 0,
            process,
            scheduler_override: None,
            sporadic_server,
            context: ExecutionContext::Vm(context),
            state: ThreadState::Ready,
            start_routine: None,
            argument: 0,
            result: 0,
            joiner: 0,
            detached,
            errno: 0,
            resource_usage: ThreadResourceUsage::default(),
            in_kernel: false,
            running_since_ns: None,
            cleanup_handlers: Vec::new(),
            tls: [0; ThreadManager::MAX_TLS_KEYS],
            signal_mask: SignalSet::EMPTY,
            pending_signals: SignalQueue::new(),
            signal_wait_set: None,
            signal_wake_pending: false,
            cancellation_requested: false,
            cancellation_enabled: true,
            sleep_deadline_ns: None,
            process_stop_wait: false,
            process_stop_wake: false,
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

fn scheduler_time_ns() -> u64 {
    crate::kernel::arch::TimerArchitecture::now_ns()
        .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns)
}

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

    fn pop_next_ready(&mut self) -> Option<Box<ListNode<Thread>>> {
        let highest_priority = self
            .ready
            .iter()
            .map(Thread::scheduling_rank)
            .max()?;
        let node = self
            .ready
            .find(|thread| thread.scheduling_rank() == highest_priority)?;
        // SAFETY: `find` returns a node owned by the ready list.
        unsafe { ListNode::remove(&mut self.ready, node) }
    }

    fn account_current_cpu_time(&mut self, now_ns: u64) {
        if let Some(thread) = self.busy.front_mut() {
            if let Some(started_ns) = thread.running_since_ns.take() {
                let elapsed_ns = now_ns.saturating_sub(started_ns);
                thread.sporadic_server.charge(elapsed_ns, now_ns);
                if thread.in_kernel {
                    thread.resource_usage.system_cpu_time_ns = thread
                        .resource_usage
                        .system_cpu_time_ns
                        .saturating_add(elapsed_ns);
                    thread.process.add_system_cpu_time_ns(elapsed_ns);
                } else {
                    thread.resource_usage.user_cpu_time_ns = thread
                        .resource_usage
                        .user_cpu_time_ns
                        .saturating_add(elapsed_ns);
                    thread.process.add_cpu_time_ns(elapsed_ns);
                }
            }
        }
    }

    fn refresh_sporadic_servers(&mut self, now_ns: u64) {
        for thread in self.ready.iter_mut() {
            thread.sporadic_server.replenish_due(now_ns);
        }
        for thread in self.busy.iter_mut() {
            thread.sporadic_server.replenish_due(now_ns);
        }
    }

    pub(crate) fn initialize() -> bool {
        let Ok(process) = Process::new() else {
            return false;
        };
        process.thread_started();
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
        main_thread.running_since_ns = Some(scheduler_time_ns());
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
        Self::create_thread_in_process_with_scheduling(
            process,
            stack_size,
            start_routine,
            argument,
            detached,
            None,
        )
    }

    pub(crate) fn create_thread_with_scheduling(
        stack_size: usize,
        start_routine: StartRoutine,
        argument: *mut c_void,
        detached: bool,
        scheduling: SchedulingParameters,
    ) -> Result<usize, Error> {
        let process = Self::current_process()?;
        Self::create_thread_in_process_with_scheduling(
            process,
            stack_size,
            start_routine,
            argument,
            detached,
            Some(scheduling),
        )
    }

    fn create_thread_in_process_with_scheduling(
        process: Arc<Process>,
        stack_size: usize,
        start_routine: StartRoutine,
        argument: *mut c_void,
        detached: bool,
        scheduling: Option<SchedulingParameters>,
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
        if let Some(scheduling) = scheduling {
            thread.scheduler_override = Some(scheduling);
            thread.sporadic_server.configure(scheduling);
        }
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
        Process::new_child_in_namespace_set_with_credentials(
            &parent,
            parent.namespace_set(),
            parent.credentials()?,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn create_container_process() -> Result<Arc<Process>, Error> {
        let parent = Self::current_process()?;
        Process::new_child_in_namespace_set_with_credentials(
            &parent,
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
        if let Some(mask) = manager
            .busy
            .front()
            .filter(|current| current.process.pid() == thread.process.pid())
            .map(|current| current.signal_mask)
        {
            thread.signal_mask = mask;
        }
        match manager.thread_nodes.insert(id, node_address) {
            Ok(None) => {}
            Ok(Some(previous)) => {
                let _ = manager.thread_nodes.insert(id, previous);
                return Err(Error::Again);
            }
            Err(_) => return Err(Error::NoMem),
        }
        thread.process.thread_started();
        manager.ready.push_back(thread);
        Ok(id)
    }

    pub(crate) fn switch_current(next_state: ThreadState) -> bool {
        Self::switch_current_inner(
            next_state,
            false,
            true,
            next_state == ThreadState::Blocked,
        )
    }

    pub(crate) fn yield_current() -> bool {
        Self::switch_current_inner(ThreadState::Ready, false, true, true)
    }

    pub(crate) fn switch_current_for_signal() -> bool {
        Self::switch_current_inner(ThreadState::Blocked, true, true, true)
    }

    fn switch_current_inner(
        next_state: ThreadState,
        check_signal_wake: bool,
        check_process_stopped: bool,
        voluntary: bool,
    ) -> bool {
        let interrupt_state = crate::kernel::arch::InterruptArchitecture::disable_interrupts();
        if !matches!(next_state, ThreadState::Ready | ThreadState::Blocked) {
            crate::kernel::arch::InterruptArchitecture::restore_interrupts(interrupt_state);
            return false;
        }

        let context = {
            let mut manager = THREAD_MANAGER.lock();
            if !manager.initialized
                || manager.ready.is_empty()
                || (check_signal_wake
                    && manager
                        .busy
                        .front()
                        .is_some_and(|thread| {
                            thread.signal_wake_pending || thread.process_stop_wake
                        }))
            {
                None
            } else {
                let now_ns = scheduler_time_ns();
                manager.account_current_cpu_time(now_ns);
                manager.refresh_sporadic_servers(now_ns);
                if let Some(mut current) = manager.busy.pop_front() {
                    current.process.record_context_switch(voluntary);
                let old_context = current.context.kernel_mut() as *mut kernel::Context;
                current.state = next_state;
                if next_state == ThreadState::Ready {
                    manager.ready.push_back(current);
                } else {
                    manager.busy.push_back(current);
                }

                if let Some(mut next) = manager.pop_next_ready() {
                    next.state = ThreadState::Running;
                    next.running_since_ns = Some(now_ns);
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
        Self::exit_if_process_terminated();
        if check_process_stopped {
            Self::wait_if_process_stopped();
        }
        true
    }

    unsafe fn switch_threads(context: SwitchContext) {
        unsafe {
            (&mut *context.old_context).restore(&*context.new_context);
        }
    }

    pub(crate) fn timer_tick() {
        let now_ns = crate::kernel::arch::TimerArchitecture::now_ns()
            .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns);
        {
            let mut manager = THREAD_MANAGER.lock();
            manager.account_current_cpu_time(now_ns);
            manager.refresh_sporadic_servers(now_ns);
            if let Some(thread) = manager.busy.front_mut() {
                thread.running_since_ns = Some(now_ns);
            }
        }
        Process::tick_alarms(now_ns);
        Self::wake_expired_sleepers(now_ns);
        if let Ok(process) = Self::current_process() {
            let cpu_time_ns = Self::process_cpu_time_ns(&process);
            process.check_cpu_limit(cpu_time_ns);
        }
        let should_preempt = {
            let manager = THREAD_MANAGER.lock();
            manager.busy.front().is_some_and(|current| {
                let policy = current.scheduling_configuration().policy;
                !matches!(policy, SCHED_FIFO_POLICY | SCHED_SPORADIC_POLICY)
                    || manager
                        .ready
                        .iter()
                        .any(|thread| thread.scheduling_rank() > current.scheduling_rank())
            })
        };
        if should_preempt {
            let _ = Self::switch_current(ThreadState::Ready);
        }
    }

    fn wake_expired_sleepers(now_ns: u64) {
        loop {
            let thread_id = {
                let manager = THREAD_MANAGER.lock();
                manager.busy.iter().find_map(|thread| {
                    (thread.state == ThreadState::Blocked
                        && thread.sleep_deadline_ns.is_some_and(|deadline| deadline <= now_ns))
                    .then_some(thread.id)
                })
            };
            let Some(thread_id) = thread_id else {
                return;
            };
            let mut manager = THREAD_MANAGER.lock();
            let Some(node) = manager.thread_node_by_id(thread_id) else {
                continue;
            };
            manager.thread_mut(node).sleep_deadline_ns = None;
            manager.wake_blocked(node);
        }
    }

    pub(crate) fn current_signal_context() -> Result<(usize, SignalSet), Error> {
        let manager = THREAD_MANAGER.lock();
        if !manager.initialized {
            return Err(Error::Again);
        }
        manager
            .busy
            .front()
            .map(|thread| (thread.id, thread.signal_mask))
            .ok_or(Error::Again)
    }

    pub(crate) fn set_current_signal_mask(mask: SignalSet) -> Result<SignalSet, Error> {
        let mut manager = THREAD_MANAGER.lock();
        if !manager.initialized {
            return Err(Error::Again);
        }
        let thread = manager.busy.front_mut().ok_or(Error::Again)?;
        let previous = thread.signal_mask;
        thread.signal_mask = mask.without_uncatchable();
        Ok(previous)
    }

    pub(crate) fn pending_current_signals() -> Result<SignalSet, Error> {
        let manager = THREAD_MANAGER.lock();
        if !manager.initialized {
            return Err(Error::Again);
        }
        manager
            .busy
            .front()
            .map(|thread| thread.pending_signals.pending_set())
            .ok_or(Error::Again)
    }

    pub(crate) fn take_current_signal(signals: SignalSet) -> Option<SignalInfo> {
        let mut manager = THREAD_MANAGER.lock();
        let thread = manager.busy.front_mut()?;
        let information = thread.pending_signals.take(signals);
        if information.is_some() {
            thread.signal_wake_pending = false;
        }
        information
    }

    pub(crate) fn take_current_signal_wake() -> bool {
        let mut manager = THREAD_MANAGER.lock();
        let Some(thread) = manager.busy.front_mut() else {
            return false;
        };
        let pending = thread.signal_wake_pending;
        thread.signal_wake_pending = false;
        pending
    }

    fn take_current_process_stop_wake() -> bool {
        let mut manager = THREAD_MANAGER.lock();
        let Some(thread) = manager.busy.front_mut() else {
            return false;
        };
        let pending = thread.process_stop_wake;
        thread.process_stop_wake = false;
        pending
    }

    pub(crate) fn current_signal_wake_pending() -> bool {
        THREAD_MANAGER
            .lock()
            .busy
            .front()
            .is_some_and(|thread| thread.signal_wake_pending)
    }

    pub(crate) fn set_current_signal_wait_set(
        signals: Option<SignalSet>,
    ) -> Result<(), Error> {
        let mut manager = THREAD_MANAGER.lock();
        if !manager.initialized {
            return Err(Error::Again);
        }
        manager.busy.front_mut().ok_or(Error::Again)?.signal_wait_set = signals;
        Ok(())
    }

    pub(crate) fn queue_signal_by_id(
        thread_id: usize,
        information: SignalInfo,
    ) -> Result<(), Error> {
        let mut manager = THREAD_MANAGER.lock();
        let node = manager.thread_node_by_id(thread_id).ok_or(Error::Srch)?;
        let should_wake = {
            let thread = manager.thread_mut(node);
            if thread.state == ThreadState::Exited {
                return Err(Error::Srch);
            }
            thread.pending_signals.push(information)?;
            let signal = SignalSet::from_signal(information.signal_number);
            let waiting_for_signal = thread
                .signal_wait_set
                .is_some_and(|wait_set| !wait_set.intersection(signal).is_empty());
            let should_wake = thread.signal_mask.intersection(signal).is_empty()
                || waiting_for_signal;
            if should_wake {
                thread.signal_wake_pending = true;
                thread.sleep_deadline_ns = None;
            }
            should_wake
        };
        if should_wake {
            manager.wake_blocked(node);
        }
        Ok(())
    }

    pub(crate) fn notify_process_signals(process_id: i32, pending: SignalSet) {
        if pending.is_empty() {
            return;
        }
        let mut manager = THREAD_MANAGER.lock();
        let target_id = manager
            .busy
            .iter()
            .chain(manager.ready.iter())
            .find_map(|thread| {
                if thread.process.pid() != process_id || thread.state == ThreadState::Exited {
                    return None;
                }
                let deliverable = pending
                    .difference(thread.signal_mask)
                    .union(pending.intersection(
                        thread.signal_wait_set.unwrap_or(SignalSet::EMPTY),
                    ));
                (!deliverable.is_empty()).then_some(thread.id)
            });
        let Some(node) = target_id.and_then(|id| manager.thread_node_by_id(id)) else {
            return;
        };
        let thread = manager.thread_mut(node);
        thread.signal_wake_pending = true;
        thread.sleep_deadline_ns = None;
        manager.wake_blocked(node);
    }

    pub(crate) fn wake_process_threads(process_id: i32) {
        let mut manager = THREAD_MANAGER.lock();
        loop {
            let thread_id = manager
                .busy
                .iter()
                .chain(manager.ready.iter())
                .find_map(|thread| {
                    (thread.process.pid() == process_id
                        && thread.state != ThreadState::Exited
                        && !thread.signal_wake_pending)
                    .then_some(thread.id)
                });
            let Some(thread_id) = thread_id else {
                return;
            };
            let Some(node) = manager.thread_node_by_id(thread_id) else {
                continue;
            };
            let thread = manager.thread_mut(node);
            thread.signal_wake_pending = true;
            thread.sleep_deadline_ns = None;
            manager.wake_blocked(node);
        }
    }

    pub(crate) fn wake_blocked_process_threads(process_id: i32) {
        let mut manager = THREAD_MANAGER.lock();
        loop {
            let thread_id = manager.busy.iter().find_map(|thread| {
                (thread.process.pid() == process_id && thread.state == ThreadState::Blocked)
                    .then_some(thread.id)
            });
            let Some(thread_id) = thread_id else {
                return;
            };
            let Some(node) = manager.thread_node_by_id(thread_id) else {
                continue;
            };
            manager.wake_blocked(node);
        }
    }

    pub(crate) fn wake_all_blocked_threads() {
        let mut manager = THREAD_MANAGER.lock();
        loop {
            let thread_id = manager
                .busy
                .iter()
                .find_map(|thread| (thread.state == ThreadState::Blocked).then_some(thread.id));
            let Some(thread_id) = thread_id else {
                return;
            };
            let Some(node) = manager.thread_node_by_id(thread_id) else {
                continue;
            };
            manager.wake_blocked(node);
        }
    }

    fn exit_if_process_terminated() {
        let Ok(process) = Self::current_process() else {
            return;
        };
        if let Some(signal_number) = process.termination_signal() {
            Self::exit_current(signal_number as usize as *mut c_void);
        }
    }

    pub(crate) fn wait_if_process_stopped() {
        loop {
            let Ok(process) = Self::current_process() else {
                return;
            };
            if process.stopped_by_signal().is_none() {
                if let Some(thread) = THREAD_MANAGER.lock().busy.front_mut() {
                    thread.process_stop_wait = false;
                    thread.process_stop_wake = false;
                }
                return;
            }
            Self::take_current_signal_wake();
            if let Some(thread) = THREAD_MANAGER.lock().busy.front_mut() {
                thread.process_stop_wait = true;
            }
            if !Self::switch_current_inner(ThreadState::Blocked, true, false, true) {
                if Self::take_current_process_stop_wake() || Self::take_current_signal_wake() {
                    continue;
                }
                if process.stopped_by_signal().is_some() {
                    crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
                }
            }
        }
    }

    pub(crate) fn resume_process_threads(process_id: i32) {
        let mut manager = THREAD_MANAGER.lock();
        loop {
            let thread_id = manager.busy.iter().find_map(|thread| {
                (thread.process.pid() == process_id && thread.process_stop_wait)
                    .then_some(thread.id)
            });
            let Some(thread_id) = thread_id else {
                return;
            };
            let Some(node) = manager.thread_node_by_id(thread_id) else {
                continue;
            };
            manager.thread_mut(node).process_stop_wake = true;
            manager.thread_mut(node).process_stop_wait = false;
            manager.wake_blocked(node);
        }
    }

    pub(crate) fn sleep_current_until(deadline_ns: u64) -> bool {
        loop {
            let now_ns = crate::kernel::arch::TimerArchitecture::now_ns()
                .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns);
            if now_ns >= deadline_ns {
                break;
            }
            {
                let mut manager = THREAD_MANAGER.lock();
                let Some(thread) = manager.busy.front_mut() else {
                    return false;
                };
                if thread.signal_wake_pending {
                    thread.signal_wake_pending = false;
                    thread.sleep_deadline_ns = None;
                    return true;
                }
                thread.sleep_deadline_ns = Some(deadline_ns);
            }
            if !Self::switch_current_for_signal() {
                crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
            }
        }
        let mut manager = THREAD_MANAGER.lock();
        if let Some(thread) = manager.busy.front_mut() {
            thread.sleep_deadline_ns = None;
            thread.signal_wake_pending = false;
        }
        false
    }

    pub(crate) fn current_thread_id() -> usize {
        let manager = THREAD_MANAGER.lock();
        if manager.initialized {
            manager.busy.front().map_or(0, |thread| thread.id)
        } else {
            0
        }
    }

    pub(crate) fn current_thread_cpu_time_ns() -> u64 {
        let manager = THREAD_MANAGER.lock();
        let now_ns = scheduler_time_ns();
        manager.busy.front().map_or(0, |thread| {
            thread.resource_usage.total_cpu_time_ns().saturating_add(
                thread
                    .running_since_ns
                    .map_or(0, |started_ns| now_ns.saturating_sub(started_ns)),
            )
        })
    }

    pub(crate) fn current_process_cpu_time_ns() -> u64 {
        let manager = THREAD_MANAGER.lock();
        let Some(thread) = manager.busy.front() else {
            return 0;
        };
        let now_ns = scheduler_time_ns();
        thread.process.cpu_time_ns().saturating_add(
            thread
                .running_since_ns
                .map_or(0, |started_ns| now_ns.saturating_sub(started_ns)),
        )
    }

    pub(crate) fn current_process_resource_usage_snapshot() -> Option<ResourceUsageSnapshot> {
        let manager = THREAD_MANAGER.lock();
        let thread = manager.busy.front()?;
        let mut usage = thread.process.resource_usage_snapshot();
        if let Some(started_ns) = thread.running_since_ns {
            let elapsed_ns = scheduler_time_ns().saturating_sub(started_ns);
            if thread.in_kernel {
                usage.system_cpu_time_ns = usage.system_cpu_time_ns.saturating_add(elapsed_ns);
            } else {
                usage.user_cpu_time_ns = usage.user_cpu_time_ns.saturating_add(elapsed_ns);
            }
        }
        Some(usage)
    }

    pub(crate) fn begin_syscall_accounting() {
        let mut manager = THREAD_MANAGER.lock();
        if !manager.initialized {
            return;
        }
        let now_ns = scheduler_time_ns();
        manager.account_current_cpu_time(now_ns);
        if let Some(thread) = manager.busy.front_mut() {
            thread.in_kernel = true;
            thread.running_since_ns = Some(now_ns);
        }
    }

    pub(crate) fn end_syscall_accounting() {
        let mut manager = THREAD_MANAGER.lock();
        if !manager.initialized {
            return;
        }
        let now_ns = scheduler_time_ns();
        manager.account_current_cpu_time(now_ns);
        if let Some(thread) = manager.busy.front_mut() {
            thread.in_kernel = false;
            thread.running_since_ns = Some(now_ns);
        }
    }

    pub(crate) fn process_cpu_time_ns(process: &Process) -> u64 {
        let manager = THREAD_MANAGER.lock();
        let now_ns = scheduler_time_ns();
        process.cpu_time_ns().saturating_add(
            manager
                .busy
                .front()
                .filter(|thread| thread.process.pid() == process.pid())
                .and_then(|thread| thread.running_since_ns)
                .map_or(0, |started_ns| now_ns.saturating_sub(started_ns)),
        )
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

    pub(crate) fn thread_belongs_to_process(thread_id: usize, process_id: i32) -> bool {
        let manager = THREAD_MANAGER.lock();
        let Some(node) = manager.thread_node_by_id(thread_id) else {
            return false;
        };
        let thread = manager.thread_ref(node);
        thread.state != ThreadState::Exited && thread.process.pid() == process_id
    }

    pub(crate) fn request_cancellation(
        thread_id: usize,
        process_id: i32,
    ) -> Result<(), Error> {
        let mut manager = THREAD_MANAGER.lock();
        let node = manager.thread_node_by_id(thread_id).ok_or(Error::Srch)?;
        let state = {
            let thread = manager.thread_ref(node);
            if thread.state == ThreadState::Exited || thread.process.pid() != process_id {
                return Err(Error::Srch);
            }
            thread.state
        };
        manager.thread_mut(node).cancellation_requested = true;
        if state == ThreadState::Blocked {
            manager.wake_blocked(node);
        }
        Ok(())
    }

    pub(crate) fn set_current_cancellation_enabled(enabled: bool) -> Result<bool, Error> {
        let mut manager = THREAD_MANAGER.lock();
        let thread = manager.busy.front_mut().ok_or(Error::Srch)?;
        let previous = thread.cancellation_enabled;
        thread.cancellation_enabled = enabled;
        Ok(previous)
    }

    pub(crate) fn cancellation_requested_current() -> bool {
        THREAD_MANAGER.lock().busy.front().is_some_and(|thread| {
            thread.cancellation_enabled && thread.cancellation_requested
        })
    }

    pub(crate) fn push_cleanup_handler(
        routine: unsafe extern "C" fn(*mut c_void),
        argument: *mut c_void,
    ) -> Result<(), Error> {
        let mut manager = THREAD_MANAGER.lock();
        let thread = manager.busy.front_mut().ok_or(Error::Srch)?;
        thread.cleanup_handlers.try_reserve(1).map_err(|_| Error::NoMem)?;
        thread.cleanup_handlers.push(CleanupHandler {
            routine,
            argument: argument as usize,
        });
        Ok(())
    }

    pub(crate) fn pop_cleanup_handler(execute: bool) -> Result<(), Error> {
        let handler = THREAD_MANAGER
            .lock()
            .busy
            .front_mut()
            .and_then(|thread| thread.cleanup_handlers.pop())
            .ok_or(Error::Inval)?;
        if execute {
            unsafe { (handler.routine)(handler.argument as *mut c_void) };
        }
        Ok(())
    }

    pub(crate) fn run_cleanup_handlers_current() {
        loop {
            let handler = THREAD_MANAGER
                .lock()
                .busy
                .front_mut()
                .and_then(|thread| thread.cleanup_handlers.pop());
            let Some(handler) = handler else {
                return;
            };
            unsafe { (handler.routine)(handler.argument as *mut c_void) };
        }
    }

    pub(crate) fn run_tls_destructors_current() {
        const DESTRUCTOR_ITERATIONS: usize = 4;
        for _ in 0..DESTRUCTOR_ITERATIONS {
            for index in 0..Self::MAX_TLS_KEYS {
                let destructor = {
                    let mut manager = THREAD_MANAGER.lock();
                    let key = manager.tls_keys[index];
                    let Some(thread) = manager.busy.front_mut() else {
                        return;
                    };
                    let value = thread.tls[index];
                    if !key.active || value == 0 {
                        None
                    } else {
                        thread.tls[index] = 0;
                        key.destructor.map(|destructor| (destructor, value))
                    }
                };
                if let Some((destructor, value)) = destructor {
                    unsafe { destructor(value as *mut c_void) };
                }
            }
        }
    }

    pub(crate) fn cancel_current_if_requested() {
        let should_cancel = {
            let mut manager = THREAD_MANAGER.lock();
            let Some(thread) = manager.busy.front_mut() else {
                return;
            };
            if thread.cancellation_enabled && thread.cancellation_requested {
                thread.cancellation_requested = false;
                true
            } else {
                false
            }
        };
        if should_cancel {
            Self::run_cleanup_handlers_current();
            Self::run_tls_destructors_current();
            Self::exit_current(usize::MAX as *mut c_void);
        }
    }

    pub(crate) fn scheduling_parameters_for_thread(
        thread_id: usize,
    ) -> Result<(i32, i32), Error> {
        let scheduling = Self::scheduling_configuration_for_thread(thread_id)?;
        Ok((scheduling.policy, scheduling.priority))
    }

    pub(crate) fn scheduling_configuration_for_thread(
        thread_id: usize,
    ) -> Result<SchedulingParameters, Error> {
        let manager = THREAD_MANAGER.lock();
        let node = manager.thread_node_by_id(thread_id).ok_or(Error::Srch)?;
        let thread = manager.thread_ref(node);
        if thread.state == ThreadState::Exited {
            return Err(Error::Srch);
        }
        Ok(thread.scheduling_configuration())
    }

    pub(crate) fn set_scheduling_parameters_for_thread(
        thread_id: usize,
        policy: i32,
        priority: i32,
    ) -> Result<(), Error> {
        Self::set_scheduling_configuration_for_thread(
            thread_id,
            SchedulingParameters {
                policy,
                priority,
                sporadic: None,
            },
        )
    }

    pub(crate) fn set_scheduling_configuration_for_thread(
        thread_id: usize,
        scheduling: SchedulingParameters,
    ) -> Result<(), Error> {
        let now_ns = scheduler_time_ns();
        let mut manager = THREAD_MANAGER.lock();
        manager.account_current_cpu_time(now_ns);
        if let Some(current) = manager.busy.front_mut() {
            current.running_since_ns = Some(now_ns);
        }
        let node = manager.thread_node_by_id(thread_id).ok_or(Error::Srch)?;
        let thread = manager.thread_mut(node);
        if thread.state == ThreadState::Exited {
            return Err(Error::Srch);
        }
        thread.scheduler_override = Some(scheduling);
        thread.sporadic_server.configure(scheduling);
        Ok(())
    }

    pub(crate) fn set_process_scheduling_configuration(
        process: &Process,
        scheduling: SchedulingParameters,
    ) {
        let process_id = process.pid();
        let now_ns = scheduler_time_ns();
        let mut manager = THREAD_MANAGER.lock();
        manager.account_current_cpu_time(now_ns);
        if let Some(current) = manager.busy.front_mut() {
            current.running_since_ns = Some(now_ns);
        }
        process.set_scheduling_configuration(scheduling);
        for thread in manager.busy.iter_mut() {
            if thread.process.pid() == process_id && thread.scheduler_override.is_none() {
                thread.sporadic_server.configure(scheduling);
            }
        }
        for thread in manager.ready.iter_mut() {
            if thread.process.pid() == process_id && thread.scheduler_override.is_none() {
                thread.sporadic_server.configure(scheduling);
            }
        }
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
        let (context, exited_process, exit_status) = {
            let mut manager = THREAD_MANAGER.lock();
            let now_ns = scheduler_time_ns();
            manager.account_current_cpu_time(now_ns);
            if let Some(mut current) = manager.busy.pop_front() {
                current.result = result as usize;
                current.state = ThreadState::Exited;
                let exited_process = Arc::clone(&current.process);
                let exit_status = current.result;
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

                if let Some(mut next) = manager.pop_next_ready() {
                    next.state = ThreadState::Running;
                    next.running_since_ns = Some(now_ns);
                    let context = SwitchContext {
                        old_context,
                        new_context: next.context.kernel() as *const kernel::Context,
                        new_user_kernel_stack: next.context.user_kernel_stack_pointer(),
                        new_address_space_root: next.process.address_space().root(),
                    };
                    manager.busy.push_front(next);
                    (Some(context), Some(exited_process), exit_status)
                } else {
                    (None, Some(exited_process), exit_status)
                }
            } else {
                (None, None, 0)
            }
        };
        if let Some(process) = exited_process {
            process.thread_exited(exit_status);
        }
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
        Self::exit_if_process_terminated();
        Self::wait_if_process_stopped();
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
        Self::exit_if_process_terminated();

        let result = unsafe { start_routine(argument) };
        Self::run_tls_destructors_current();
        Self::exit_current(result)
    }
}

#[cfg_attr(not(test), no_mangle)]
pub extern "C" fn __errno_location() -> *mut i32 {
    ThreadManager::errno_location()
}
