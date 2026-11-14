/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::kernel::error::Error;
pub(crate) use crate::kernel::credentials::{GroupId, UserId};
use crate::kernel::ipc::{
    CallFuture, CallTicket, Capability, CapabilityApi as _, CapabilityRights, Endpoint,
    EndpointApi as _, MessageBody, MessageBodyApi as _, ReceivedMessage,
    ReceivedMessageApi as _, ReplyCapability, ReplyCapabilityApi as _, SendFuture,
    SharedMemoryGrantApi as _, SharedMemoryMapping,
};
use crate::kernel::lib::hashmap::HashMap;
use crate::kernel::namespace::NamespaceSet;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::vfs::FileHandle;
use crate::kernel::scheduler::thread::{vm::UserAddressSpace, ThreadManager};
use crate::kernel::signal::{
    SignalAction, SignalInfo, SignalSet, SignalState, CLD_CONTINUED, CLD_EXITED, CLD_KILLED,
    CLD_STOPPED, MAX_SIGNAL_NUMBER, SIGCHLD, SIGCONT, SIGKILL, SIGSTOP, SIGXCPU,
    SA_NOCLDSTOP,
};

const INITIAL_CAPABILITY_SLOTS: usize = 32;
const FIRST_FILE_DESCRIPTOR: u32 = 3;
const CHILD_EVENT_CAPACITY: usize = 8;
pub(crate) const MAX_SUPPLEMENTARY_GROUPS: usize = 64;
static NEXT_PROCESS_ID: AtomicU32 = AtomicU32::new(1);
static PROCESS_REGISTRY: IrqSpinLock<Vec<ProcessRegistryEntry>> = IrqSpinLock::new(Vec::new());

struct ProcessRegistryEntry {
    pid: i32,
    process: Weak<Process>,
}

#[derive(Clone, Copy)]
struct ResourceLimit {
    soft: u64,
    hard: u64,
}

#[derive(Clone, Copy)]
struct ResourceLimits {
    cpu: ResourceLimit,
    file_size: ResourceLimit,
    data: ResourceLimit,
    stack: ResourceLimit,
    core: ResourceLimit,
    open_files: ResourceLimit,
    address_space: ResourceLimit,
}

impl ResourceLimits {
    fn get(&self, resource: i32) -> Option<ResourceLimit> {
        match resource {
            0 => Some(self.cpu),
            1 => Some(self.file_size),
            2 => Some(self.data),
            3 => Some(self.stack),
            4 => Some(self.core),
            5 => Some(self.open_files),
            9 => Some(self.address_space),
            _ => None,
        }
    }

    fn get_mut(&mut self, resource: i32) -> Option<&mut ResourceLimit> {
        match resource {
            0 => Some(&mut self.cpu),
            1 => Some(&mut self.file_size),
            2 => Some(&mut self.data),
            3 => Some(&mut self.stack),
            4 => Some(&mut self.core),
            5 => Some(&mut self.open_files),
            9 => Some(&mut self.address_space),
            _ => None,
        }
    }
}

const UNLIMITED_RESOURCE: ResourceLimit = ResourceLimit {
    soft: u64::MAX,
    hard: u64::MAX,
};
const DEFAULT_RESOURCE_LIMITS: ResourceLimits = ResourceLimits {
    cpu: UNLIMITED_RESOURCE,
    file_size: UNLIMITED_RESOURCE,
    data: UNLIMITED_RESOURCE,
    stack: ResourceLimit {
        soft: 8 * 1024 * 1024,
        hard: 8 * 1024 * 1024,
    },
    core: ResourceLimit { soft: 0, hard: 0 },
    open_files: ResourceLimit {
        soft: 1024,
        hard: 1024,
    },
    address_space: UNLIMITED_RESOURCE,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ResourceUsageSnapshot {
    pub(crate) user_cpu_time_ns: u64,
    pub(crate) system_cpu_time_ns: u64,
    pub(crate) max_rss_bytes: u64,
    pub(crate) minor_page_faults: u64,
    pub(crate) major_page_faults: u64,
    pub(crate) swaps: u64,
    pub(crate) input_blocks: u64,
    pub(crate) output_blocks: u64,
    pub(crate) messages_sent: u64,
    pub(crate) messages_received: u64,
    pub(crate) signals_received: u64,
    pub(crate) voluntary_context_switches: u64,
    pub(crate) involuntary_context_switches: u64,
}

impl ResourceUsageSnapshot {
    pub(crate) fn total_cpu_time_ns(self) -> u64 {
        self.user_cpu_time_ns
            .saturating_add(self.system_cpu_time_ns)
    }

    fn accumulate(&mut self, other: Self) {
        self.user_cpu_time_ns = self.user_cpu_time_ns.saturating_add(other.user_cpu_time_ns);
        self.system_cpu_time_ns = self.system_cpu_time_ns.saturating_add(other.system_cpu_time_ns);
        self.max_rss_bytes = self.max_rss_bytes.max(other.max_rss_bytes);
        self.minor_page_faults = self.minor_page_faults.saturating_add(other.minor_page_faults);
        self.major_page_faults = self.major_page_faults.saturating_add(other.major_page_faults);
        self.swaps = self.swaps.saturating_add(other.swaps);
        self.input_blocks = self.input_blocks.saturating_add(other.input_blocks);
        self.output_blocks = self.output_blocks.saturating_add(other.output_blocks);
        self.messages_sent = self.messages_sent.saturating_add(other.messages_sent);
        self.messages_received = self.messages_received.saturating_add(other.messages_received);
        self.signals_received = self.signals_received.saturating_add(other.signals_received);
        self.voluntary_context_switches = self
            .voluntary_context_switches
            .saturating_add(other.voluntary_context_switches);
        self.involuntary_context_switches = self
            .involuntary_context_switches
            .saturating_add(other.involuntary_context_switches);
    }
}

#[derive(Clone, Copy, Default)]
struct MemoryResourceUsage {
    address_space_bytes: u64,
    data_bytes: u64,
    resident_bytes: u64,
    max_rss_bytes: u64,
    minor_page_faults: u64,
    major_page_faults: u64,
}

#[derive(Clone, Copy, Default)]
struct IoResourceUsage {
    input_blocks: u64,
    output_blocks: u64,
    messages_sent: u64,
    messages_received: u64,
    swaps: u64,
}

#[derive(Clone, Copy, Default)]
struct SchedulerResourceUsage {
    voluntary_context_switches: u64,
    involuntary_context_switches: u64,
}

struct ProcessResourceUsage {
    user_cpu_time_ns: AtomicU64,
    system_cpu_time_ns: AtomicU64,
    cpu_limit_signals: AtomicU64,
    memory: IrqSpinLock<MemoryResourceUsage>,
    io: IrqSpinLock<IoResourceUsage>,
    scheduler: IrqSpinLock<SchedulerResourceUsage>,
    signals_received: AtomicU64,
    child_usage: IrqSpinLock<ResourceUsageSnapshot>,
}

impl ProcessResourceUsage {
    fn new() -> Self {
        Self {
            user_cpu_time_ns: AtomicU64::new(0),
            system_cpu_time_ns: AtomicU64::new(0),
            cpu_limit_signals: AtomicU64::new(0),
            memory: IrqSpinLock::new(MemoryResourceUsage::default()),
            io: IrqSpinLock::new(IoResourceUsage::default()),
            scheduler: IrqSpinLock::new(SchedulerResourceUsage::default()),
            signals_received: AtomicU64::new(0),
            child_usage: IrqSpinLock::new(ResourceUsageSnapshot::default()),
        }
    }

    fn total_cpu_time_ns(&self) -> u64 {
        self.user_cpu_time_ns
            .load(Ordering::Relaxed)
            .saturating_add(self.system_cpu_time_ns.load(Ordering::Relaxed))
    }

    fn snapshot(&self) -> ResourceUsageSnapshot {
        let memory = *self.memory.lock();
        let io = *self.io.lock();
        let scheduler = *self.scheduler.lock();
        ResourceUsageSnapshot {
            user_cpu_time_ns: self.user_cpu_time_ns.load(Ordering::Relaxed),
            system_cpu_time_ns: self.system_cpu_time_ns.load(Ordering::Relaxed),
            max_rss_bytes: memory.max_rss_bytes,
            minor_page_faults: memory.minor_page_faults,
            major_page_faults: memory.major_page_faults,
            swaps: io.swaps,
            input_blocks: io.input_blocks,
            output_blocks: io.output_blocks,
            messages_sent: io.messages_sent,
            messages_received: io.messages_received,
            signals_received: self.signals_received.load(Ordering::Relaxed),
            voluntary_context_switches: scheduler.voluntary_context_switches,
            involuntary_context_switches: scheduler.involuntary_context_switches,
        }
    }

    fn child_snapshot(&self) -> ResourceUsageSnapshot {
        *self.child_usage.lock()
    }

    fn add_child_usage(&self, usage: ResourceUsageSnapshot) {
        self.child_usage.lock().accumulate(usage);
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SporadicParameters {
    pub(crate) low_priority: i32,
    pub(crate) replenishment_period_ns: u64,
    pub(crate) initial_budget_ns: u64,
    pub(crate) max_replenishments: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct SchedulingParameters {
    pub(crate) policy: i32,
    pub(crate) priority: i32,
    pub(crate) sporadic: Option<SporadicParameters>,
}

impl SchedulingParameters {
    pub(crate) const OTHER: Self = Self {
        policy: 0,
        priority: 0,
        sporadic: None,
    };
}

#[derive(Clone, Copy)]
struct ProcessControlState {
    parent_pid: Option<i32>,
    process_group_id: i32,
    session_id: i32,
    controlling_terminal: bool,
    scheduling: SchedulingParameters,
    nice_priority: i32,
    resource_limits: ResourceLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ChildExitStatus {
    pub(crate) process_id: i32,
    pub(crate) process_group_id: i32,
    pub(crate) code: i32,
    pub(crate) status: i32,
    resource_usage: ResourceUsageSnapshot,
}

struct ChildProcessRecord {
    process_id: i32,
    process_group_id: i32,
    events: Vec<ChildExitStatus>,
}

struct FilesystemContext {
    current_directory: Vec<u8>,
    umask: u32,
}

impl FilesystemContext {
    fn root() -> Result<Self, Error> {
        let mut current_directory = Vec::new();
        current_directory.try_reserve_exact(1).map_err(|_| Error::NoMem)?;
        current_directory.push(b'/');
        Ok(Self {
            current_directory,
            umask: 0o022,
        })
    }

    fn try_clone(&self) -> Result<Self, Error> {
        let mut current_directory = Vec::new();
        current_directory
            .try_reserve_exact(self.current_directory.len())
            .map_err(|_| Error::NoMem)?;
        current_directory.extend_from_slice(&self.current_directory);
        Ok(Self {
            current_directory,
            umask: self.umask,
        })
    }
}

pub(crate) struct Credentials {
    real_user: UserId,
    effective_user: UserId,
    saved_user: UserId,
    real_group: GroupId,
    effective_group: GroupId,
    saved_group: GroupId,
    supplementary_groups: Vec<GroupId>,
}

impl Credentials {
    pub(crate) const fn root() -> Self {
        Self {
            real_user: UserId::ROOT,
            effective_user: UserId::ROOT,
            saved_user: UserId::ROOT,
            real_group: GroupId::ROOT,
            effective_group: GroupId::ROOT,
            saved_group: GroupId::ROOT,
            supplementary_groups: Vec::new(),
        }
    }

    pub(crate) fn new(
        real_user: UserId,
        effective_user: UserId,
        real_group: GroupId,
        effective_group: GroupId,
        supplementary_groups: Vec<GroupId>,
    ) -> Self {
        Self {
            real_user,
            effective_user,
            saved_user: effective_user,
            real_group,
            effective_group,
            saved_group: effective_group,
            supplementary_groups,
        }
    }

    pub(crate) fn reset_effective_ids(&mut self) {
        self.effective_user = self.real_user;
        self.saved_user = self.real_user;
        self.effective_group = self.real_group;
        self.saved_group = self.real_group;
    }

    pub(crate) const fn real_user(&self) -> UserId {
        self.real_user
    }

    pub(crate) const fn effective_user(&self) -> UserId {
        self.effective_user
    }

    pub(crate) const fn saved_user(&self) -> UserId {
        self.saved_user
    }

    pub(crate) const fn real_group(&self) -> GroupId {
        self.real_group
    }

    pub(crate) const fn effective_group(&self) -> GroupId {
        self.effective_group
    }

    pub(crate) const fn saved_group(&self) -> GroupId {
        self.saved_group
    }

    pub(crate) fn supplementary_groups(&self) -> &[GroupId] {
        &self.supplementary_groups
    }

    pub(crate) fn is_member_of(&self, group: GroupId) -> bool {
        self.effective_group == group || self.supplementary_groups.contains(&group)
    }

    pub(crate) const fn is_root(&self) -> bool {
        self.effective_user.raw() == 0
    }

    fn try_clone(&self) -> Result<Self, Error> {
        let mut supplementary_groups = Vec::new();
        supplementary_groups
            .try_reserve_exact(self.supplementary_groups.len())
            .map_err(|_| Error::NoMem)?;
        supplementary_groups.extend_from_slice(&self.supplementary_groups);
        Ok(Self {
            real_user: self.real_user,
            effective_user: self.effective_user,
            saved_user: self.saved_user,
            real_group: self.real_group,
            effective_group: self.effective_group,
            saved_group: self.saved_group,
            supplementary_groups,
        })
    }

    fn set_user_id(&mut self, user: UserId) -> Result<(), Error> {
        if self.is_root() {
            self.real_user = user;
            self.effective_user = user;
            self.saved_user = user;
        } else if user == self.real_user || user == self.saved_user {
            self.effective_user = user;
        } else {
            return Err(Error::Perm);
        }
        Ok(())
    }

    fn set_effective_user(&mut self, user: UserId) -> Result<(), Error> {
        if !self.is_root()
            && user != self.real_user
            && user != self.effective_user
            && user != self.saved_user
        {
            return Err(Error::Perm);
        }
        self.effective_user = user;
        Ok(())
    }

    fn set_real_and_effective_user(
        &mut self,
        real_user: Option<UserId>,
        effective_user: Option<UserId>,
    ) -> Result<(), Error> {
        if !self.is_root() {
            if real_user.is_some_and(|user| user != self.real_user && user != self.effective_user)
                || effective_user
                    .is_some_and(|user| {
                        user != self.real_user
                            && user != self.effective_user
                            && user != self.saved_user
                    })
            {
                return Err(Error::Perm);
            }
        }
        let real_user_was_set = real_user.is_some();
        let effective_user_was_set = effective_user.is_some();
        if let Some(user) = real_user {
            self.real_user = user;
        }
        if let Some(user) = effective_user {
            self.effective_user = user;
        }
        if real_user_was_set
            || (effective_user_was_set && self.effective_user != self.real_user)
        {
            self.saved_user = self.effective_user;
        }
        Ok(())
    }

    fn set_all_user_ids(
        &mut self,
        real_user: Option<UserId>,
        effective_user: Option<UserId>,
        saved_user: Option<UserId>,
    ) -> Result<(), Error> {
        if !self.is_root()
            && [real_user, effective_user, saved_user]
                .into_iter()
                .flatten()
                .any(|user| {
                    user != self.real_user
                        && user != self.effective_user
                        && user != self.saved_user
                })
        {
            return Err(Error::Perm);
        }
        if let Some(user) = real_user {
            self.real_user = user;
        }
        if let Some(user) = effective_user {
            self.effective_user = user;
        }
        if let Some(user) = saved_user {
            self.saved_user = user;
        }
        Ok(())
    }

    fn set_group_id(&mut self, group: GroupId) -> Result<(), Error> {
        if self.is_root() {
            self.real_group = group;
            self.effective_group = group;
            self.saved_group = group;
        } else if group == self.real_group || group == self.saved_group {
            self.effective_group = group;
        } else {
            return Err(Error::Perm);
        }
        Ok(())
    }

    fn set_effective_group(&mut self, group: GroupId) -> Result<(), Error> {
        if !self.is_root()
            && group != self.real_group
            && group != self.effective_group
            && group != self.saved_group
        {
            return Err(Error::Perm);
        }
        self.effective_group = group;
        Ok(())
    }

    fn set_real_and_effective_group(
        &mut self,
        real_group: Option<GroupId>,
        effective_group: Option<GroupId>,
    ) -> Result<(), Error> {
        if !self.is_root() {
            if real_group
                .is_some_and(|group| group != self.real_group && group != self.effective_group)
                || effective_group
                    .is_some_and(|group| {
                        group != self.real_group
                            && group != self.effective_group
                            && group != self.saved_group
                    })
            {
                return Err(Error::Perm);
            }
        }
        let real_group_was_set = real_group.is_some();
        let effective_group_was_set = effective_group.is_some();
        if let Some(group) = real_group {
            self.real_group = group;
        }
        if let Some(group) = effective_group {
            self.effective_group = group;
        }
        if real_group_was_set
            || (effective_group_was_set && self.effective_group != self.real_group)
        {
            self.saved_group = self.effective_group;
        }
        Ok(())
    }

    fn set_all_group_ids(
        &mut self,
        real_group: Option<GroupId>,
        effective_group: Option<GroupId>,
        saved_group: Option<GroupId>,
    ) -> Result<(), Error> {
        if !self.is_root()
            && [real_group, effective_group, saved_group]
                .into_iter()
                .flatten()
                .any(|group| {
                    group != self.real_group
                        && group != self.effective_group
                        && group != self.saved_group
                })
        {
            return Err(Error::Perm);
        }
        if let Some(group) = real_group {
            self.real_group = group;
        }
        if let Some(group) = effective_group {
            self.effective_group = group;
        }
        if let Some(group) = saved_group {
            self.saved_group = group;
        }
        Ok(())
    }

    fn set_supplementary_groups(&mut self, groups: Vec<GroupId>) -> Result<(), Error> {
        if !self.is_root() {
            return Err(Error::Perm);
        }
        self.supplementary_groups = groups;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CapabilityHandle(u64);

impl CapabilityHandle {
    pub(crate) const fn raw(self) -> u64 {
        self.0
    }

    fn new(index: usize, generation: u32) -> Result<Self, Error> {
        let index = u32::try_from(index).map_err(|_| Error::Mfile)?;
        Ok(Self((u64::from(generation) << 32) | u64::from(index)))
    }

    fn parts(self) -> (usize, u32) {
        (self.0 as u32 as usize, (self.0 >> 32) as u32)
    }
}

struct CapabilitySlot {
    generation: u32,
    capability: Option<Capability>,
}

impl CapabilitySlot {
    const fn empty() -> Self {
        Self {
            generation: 0,
            capability: None,
        }
    }
}

struct CapabilityTable {
    slots: Vec<CapabilitySlot>,
    free_slots: Vec<usize>,
}

impl CapabilityTable {
    fn new() -> Result<Self, Error> {
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(INITIAL_CAPABILITY_SLOTS)
            .map_err(|_| Error::NoMem)?;
        slots.resize_with(INITIAL_CAPABILITY_SLOTS, CapabilitySlot::empty);
        let mut free_slots = Vec::new();
        free_slots
            .try_reserve_exact(INITIAL_CAPABILITY_SLOTS)
            .map_err(|_| Error::NoMem)?;
        free_slots.extend((0..INITIAL_CAPABILITY_SLOTS).rev());
        Ok(Self { slots, free_slots })
    }

    fn insert(&mut self, capability: Capability) -> Result<CapabilityHandle, Error> {
        if self.free_slots.is_empty() {
            self.slots.try_reserve(1).map_err(|_| Error::NoMem)?;
            self.free_slots.try_reserve(1).map_err(|_| Error::NoMem)?;
            let index = self.slots.len();
            self.slots.push(CapabilitySlot::empty());
            self.free_slots.push(index);
        }
        let index = self.free_slots.pop().ok_or(Error::Proto)?;
        let slot = &mut self.slots[index];
        let generation = slot.generation.wrapping_add(1).max(1);
        let handle = CapabilityHandle::new(index, generation)?;
        slot.generation = generation;
        slot.capability = Some(capability);
        Ok(handle)
    }

    fn get(&self, handle: CapabilityHandle) -> Result<&Capability, Error> {
        let (index, generation) = handle.parts();
        let slot = self.slots.get(index).ok_or(Error::Badf)?;
        if slot.generation != generation {
            return Err(Error::Badf);
        }
        slot.capability.as_ref().ok_or(Error::Badf)
    }

    fn remove(&mut self, handle: CapabilityHandle) -> Result<Capability, Error> {
        let (index, generation) = handle.parts();
        let slot = self.slots.get_mut(index).ok_or(Error::Badf)?;
        if slot.generation != generation {
            return Err(Error::Badf);
        }
        self.free_slots.try_reserve(1).map_err(|_| Error::NoMem)?;
        let capability = slot.capability.take().ok_or(Error::Badf)?;
        self.free_slots.push(index);
        Ok(capability)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct KernelObjectRef(u64);

impl KernelObjectRef {
    pub(crate) const fn from_raw(reference: u64) -> Self {
        Self(reference)
    }

    pub(crate) const fn raw(self) -> u64 {
        self.0
    }

    pub(crate) fn from_arc<T>(object: &Arc<T>) -> Self {
        Self(Arc::as_ptr(object) as usize as u64)
    }
}

pub(crate) struct KernelObject {
    file: IrqSpinLock<Option<FileHandle>>,
}

impl KernelObject {
    fn new(file: FileHandle) -> Self {
        Self {
            file: IrqSpinLock::new(Some(file)),
        }
    }

    pub(crate) fn take_file(&self) -> Result<FileHandle, Error> {
        self.file.lock().take().ok_or(Error::Busy)
    }

    pub(crate) fn restore_file(&self, file: FileHandle) -> Result<(), Error> {
        let mut slot = self.file.lock();
        if slot.is_some() {
            return Err(Error::Busy);
        }
        *slot = Some(file);
        Ok(())
    }
}

struct KernelObjectTable {
    objects: HashMap<KernelObjectRef, Arc<KernelObject>>,
}

impl KernelObjectTable {
    const fn new() -> Self {
        Self {
            objects: HashMap::new(),
        }
    }

    fn insert(&mut self, file: FileHandle) -> Result<KernelObjectRef, Error> {
        let object = Arc::new(KernelObject::new(file));
        let reference = KernelObjectRef::from_arc(&object);
        match self.objects.insert(reference, object) {
            Ok(None) => Ok(reference),
            Ok(Some(previous)) => {
                let _ = self.objects.insert(reference, previous);
                Err(Error::Exist)
            }
            Err((_reference, _object)) => Err(Error::NoMem),
        }
    }

    fn get(&self, reference: KernelObjectRef) -> Result<Arc<KernelObject>, Error> {
        self.objects.get(&reference).cloned().ok_or(Error::Badf)
    }

    fn remove(&mut self, reference: KernelObjectRef) -> Result<Arc<KernelObject>, Error> {
        self.objects.remove(&reference).ok_or(Error::Badf)
    }
}

struct FileDescriptorTable {
    entries: Vec<Option<KernelObjectRef>>,
}

impl FileDescriptorTable {
    const fn new() -> Self {
        Self { entries: Vec::new() }
    }

    fn descriptor(index: usize) -> Result<u32, Error> {
        u32::try_from(index)
            .ok()
            .and_then(|index| index.checked_add(FIRST_FILE_DESCRIPTOR))
            .ok_or(Error::Mfile)
    }

    fn insert(&mut self, object: KernelObjectRef) -> Result<u32, Error> {
        if let Some(index) = self.entries.iter().position(Option::is_none) {
            let descriptor = Self::descriptor(index)?;
            self.entries[index] = Some(object);
            return Ok(descriptor);
        }

        let index = self.entries.len();
        let descriptor = Self::descriptor(index)?;
        self.entries.try_reserve(1).map_err(|_| Error::NoMem)?;
        self.entries.push(Some(object));
        Ok(descriptor)
    }

    fn get(&self, descriptor: u32) -> Result<KernelObjectRef, Error> {
        let index = descriptor
            .checked_sub(FIRST_FILE_DESCRIPTOR)
            .ok_or(Error::Badf)? as usize;
        self.entries
            .get(index)
            .copied()
            .flatten()
            .ok_or(Error::Badf)
    }

    fn remove(&mut self, descriptor: u32) -> Result<KernelObjectRef, Error> {
        let index = descriptor
            .checked_sub(FIRST_FILE_DESCRIPTOR)
            .ok_or(Error::Badf)? as usize;
        self.entries
            .get_mut(index)
            .and_then(Option::take)
            .ok_or(Error::Badf)
    }
}

pub(crate) struct Process {
    pid: i32,
    active_threads: AtomicUsize,
    resource_usage: ProcessResourceUsage,
    exit_notified: AtomicBool,
    control: IrqSpinLock<ProcessControlState>,
    pty_terminal: IrqSpinLock<Option<Arc<crate::kernel::tty::Pty>>>,
    namespace_set: NamespaceSet,
    address_space: IrqSpinLock<UserAddressSpace>,
    signals: IrqSpinLock<SignalState>,
    credentials: IrqSpinLock<Credentials>,
    capabilities: IrqSpinLock<CapabilityTable>,
    kernel_objects: IrqSpinLock<KernelObjectTable>,
    file_descriptors: IrqSpinLock<FileDescriptorTable>,
    children: IrqSpinLock<Vec<ChildProcessRecord>>,
    filesystem: IrqSpinLock<FilesystemContext>,
}

impl Process {
    pub(crate) fn new() -> Result<Arc<Self>, Error> {
        Self::new_in_namespace_set(NamespaceSet::root())
    }

    pub(crate) fn new_in_namespace_set(namespace_set: NamespaceSet) -> Result<Arc<Self>, Error> {
        Self::new_in_namespace_set_with_credentials(namespace_set, Credentials::root())
    }

    pub(crate) fn new_in_namespace_set_with_credentials(
        namespace_set: NamespaceSet,
        credentials: Credentials,
    ) -> Result<Arc<Self>, Error> {
        Self::new_with_control(namespace_set, credentials, None, None, None, None)
    }

    pub(crate) fn new_child_in_namespace_set_with_credentials(
        parent: &Arc<Self>,
        namespace_set: NamespaceSet,
        credentials: Credentials,
    ) -> Result<Arc<Self>, Error> {
        let parent_control = *parent.control.lock();
        let filesystem_context = parent.filesystem.lock().try_clone()?;
        let pty_terminal = parent.pty_terminal.lock().as_ref().map(Arc::clone);
        let child = Self::new_with_control(
            namespace_set,
            credentials,
            Some(ProcessControlState {
                parent_pid: Some(parent.pid),
                ..parent_control
            }),
            Some(filesystem_context),
            pty_terminal,
            None,
        )?;
        parent.register_child(&child)?;
        Ok(child)
    }

    pub(crate) fn new_child_with_address_space(
        parent: &Arc<Self>,
        namespace_set: NamespaceSet,
        credentials: Credentials,
        address_space: UserAddressSpace,
    ) -> Result<Arc<Self>, Error> {
        let parent_control = *parent.control.lock();
        let filesystem_context = parent.filesystem.lock().try_clone()?;
        let pty_terminal = parent.pty_terminal.lock().as_ref().map(Arc::clone);
        let child = Self::new_with_control(
            namespace_set,
            credentials,
            Some(ProcessControlState {
                parent_pid: Some(parent.pid),
                ..parent_control
            }),
            Some(filesystem_context),
            pty_terminal,
            Some(address_space),
        )?;
        parent.register_child(&child)?;
        Ok(child)
    }

    fn new_with_control(
        namespace_set: NamespaceSet,
        credentials: Credentials,
        inherited_control: Option<ProcessControlState>,
        inherited_filesystem: Option<FilesystemContext>,
        inherited_pty_terminal: Option<Arc<crate::kernel::tty::Pty>>,
        prepared_address_space: Option<UserAddressSpace>,
    ) -> Result<Arc<Self>, Error> {
        let pid = Self::allocate_pid()?;
        let control = inherited_control.unwrap_or(ProcessControlState {
            parent_pid: None,
            process_group_id: pid,
            session_id: pid,
            controlling_terminal: false,
            scheduling: SchedulingParameters::OTHER,
            nice_priority: 0,
            resource_limits: DEFAULT_RESOURCE_LIMITS,
        });
        let address_space = match prepared_address_space {
            Some(address_space) => address_space,
            None => UserAddressSpace::new()?,
        };
        let capabilities = CapabilityTable::new()?;
        let filesystem = match inherited_filesystem {
            Some(filesystem) => filesystem,
            None => FilesystemContext::root()?,
        };
        let process = Arc::new(Self {
            pid,
            active_threads: AtomicUsize::new(0),
            resource_usage: ProcessResourceUsage::new(),
            exit_notified: AtomicBool::new(false),
            control: IrqSpinLock::new(control),
            pty_terminal: IrqSpinLock::new(inherited_pty_terminal),
            namespace_set,
            address_space: IrqSpinLock::new(address_space),
            signals: IrqSpinLock::new(SignalState::new()),
            credentials: IrqSpinLock::new(credentials),
            capabilities: IrqSpinLock::new(capabilities),
            kernel_objects: IrqSpinLock::new(KernelObjectTable::new()),
            file_descriptors: IrqSpinLock::new(FileDescriptorTable::new()),
            children: IrqSpinLock::new(Vec::new()),
            filesystem: IrqSpinLock::new(filesystem),
        });
        Self::register(&process)?;
        Ok(process)
    }



    fn allocate_pid() -> Result<i32, Error> {
        NEXT_PROCESS_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                (next <= i32::MAX as u32).then_some(next + 1)
            })
            .ok()
            .and_then(|pid| i32::try_from(pid).ok())
            .ok_or(Error::NoSpc)
    }

    fn register(process: &Arc<Self>) -> Result<(), Error> {
        let mut registry = PROCESS_REGISTRY.lock();
        registry.retain(|entry| entry.process.strong_count() != 0);
        let index = registry
            .binary_search_by_key(&process.pid, |entry| entry.pid)
            .unwrap_or_else(|index| index);
        if registry.get(index).is_some_and(|entry| entry.pid == process.pid) {
            return Err(Error::Exist);
        }
        registry.try_reserve(1).map_err(|_| Error::NoMem)?;
        registry.insert(
            index,
            ProcessRegistryEntry {
                pid: process.pid,
                process: Arc::downgrade(process),
            },
        );
        let mut control = process.control.lock();
        if control.parent_pid.is_none() {
            control.controlling_terminal =
                crate::kernel::tty::attach_initial_session(
                    control.session_id,
                    control.process_group_id,
                );
        }
        Ok(())
    }

    fn lookup_registered(registry: &[ProcessRegistryEntry], pid: i32) -> Option<Arc<Self>> {
        let index = registry.binary_search_by_key(&pid, |entry| entry.pid).ok()?;
        registry[index].process.upgrade()
    }

    pub(crate) fn lookup(pid: i32) -> Result<Arc<Self>, Error> {
        if pid <= 0 {
            return Err(Error::Srch);
        }
        let mut registry = PROCESS_REGISTRY.lock();
        registry.retain(|entry| entry.process.strong_count() != 0);
        Self::lookup_registered(&registry, pid).ok_or(Error::Srch)
    }

    pub(crate) fn registered_processes() -> Result<Vec<Arc<Self>>, Error> {
        let mut registry = PROCESS_REGISTRY.lock();
        registry.retain(|entry| entry.process.strong_count() != 0);
        let mut processes = Vec::new();
        processes
            .try_reserve_exact(registry.len())
            .map_err(|_| Error::NoMem)?;
        processes.extend(registry.iter().filter_map(|entry| entry.process.upgrade()));
        Ok(processes)
    }

    pub(crate) fn may_change_scheduling(&self, caller: &Process) -> bool {
        self.may_inspect_process(caller)
    }

    pub(crate) fn may_inspect_process(&self, caller: &Process) -> bool {
        caller.effective_user_is_root() || self.user_ids()[0] == caller.user_ids()[0]
    }

    fn process_group_exists(
        registry: &[ProcessRegistryEntry],
        process_group_id: i32,
        session_id: i32,
    ) -> bool {
        registry.iter().any(|entry| {
            entry.process.upgrade().is_some_and(|process| {
                let control = process.control.lock();
                control.process_group_id == process_group_id && control.session_id == session_id
            })
        })
    }

    fn process_group_exists_any(registry: &[ProcessRegistryEntry], process_group_id: i32) -> bool {
        registry.iter().any(|entry| {
            entry.process.upgrade().is_some_and(|process| {
                process.control.lock().process_group_id == process_group_id
            })
        })
    }

    pub(crate) const fn pid(&self) -> i32 {
        self.pid
    }

    pub(crate) fn add_cpu_time_ns(&self, elapsed_ns: u64) {
        self.resource_usage
            .user_cpu_time_ns
            .fetch_add(elapsed_ns, Ordering::Relaxed);
    }

    pub(crate) fn add_system_cpu_time_ns(&self, elapsed_ns: u64) {
        self.resource_usage
            .system_cpu_time_ns
            .fetch_add(elapsed_ns, Ordering::Relaxed);
    }

    pub(crate) fn cpu_time_ns(&self) -> u64 {
        self.resource_usage.total_cpu_time_ns()
    }

    pub(crate) fn child_cpu_time_ns(&self) -> u64 {
        self.resource_usage.child_snapshot().total_cpu_time_ns()
    }

    pub(crate) fn resource_usage_snapshot(&self) -> ResourceUsageSnapshot {
        self.resource_usage.snapshot()
    }

    pub(crate) fn child_resource_usage_snapshot(&self) -> ResourceUsageSnapshot {
        self.resource_usage.child_snapshot()
    }

    pub(crate) fn try_reserve_memory(&self, bytes: u64) -> Result<(), Error> {
        let control = self.control.lock();
        let mut usage = self.resource_usage.memory.lock();
        let address_space_bytes = usage
            .address_space_bytes
            .checked_add(bytes)
            .ok_or(Error::NoMem)?;
        let data_bytes = usage.data_bytes.checked_add(bytes).ok_or(Error::NoMem)?;
        if address_space_bytes > control.resource_limits.address_space.soft
            || data_bytes > control.resource_limits.data.soft
        {
            return Err(Error::NoMem);
        }
        usage.address_space_bytes = address_space_bytes;
        usage.data_bytes = data_bytes;
        Ok(())
    }

    pub(crate) fn release_memory(&self, address_space_bytes: u64, data_bytes: u64, resident_bytes: u64) {
        let mut usage = self.resource_usage.memory.lock();
        usage.address_space_bytes = usage.address_space_bytes.saturating_sub(address_space_bytes);
        usage.data_bytes = usage.data_bytes.saturating_sub(data_bytes);
        usage.resident_bytes = usage.resident_bytes.saturating_sub(resident_bytes);
    }

    pub(crate) fn record_page_fault(&self, major: bool) {
        const PAGE_SIZE: u64 = 4096;
        let mut usage = self.resource_usage.memory.lock();
        if major {
            usage.major_page_faults = usage.major_page_faults.saturating_add(1);
        } else {
            usage.minor_page_faults = usage.minor_page_faults.saturating_add(1);
        }
        usage.resident_bytes = usage.resident_bytes.saturating_add(PAGE_SIZE);
        usage.max_rss_bytes = usage.max_rss_bytes.max(usage.resident_bytes);
    }

    pub(crate) fn record_input_blocks(&self, blocks: u64) {
        let mut usage = self.resource_usage.io.lock();
        usage.input_blocks = usage.input_blocks.saturating_add(blocks);
    }

    pub(crate) fn record_output_blocks(&self, blocks: u64) {
        let mut usage = self.resource_usage.io.lock();
        usage.output_blocks = usage.output_blocks.saturating_add(blocks);
    }

    pub(crate) fn record_message_sent(&self) {
        let mut usage = self.resource_usage.io.lock();
        usage.messages_sent = usage.messages_sent.saturating_add(1);
    }

    pub(crate) fn record_message_received(&self) {
        let mut usage = self.resource_usage.io.lock();
        usage.messages_received = usage.messages_received.saturating_add(1);
    }

    pub(crate) fn record_signal_received(&self) {
        self.resource_usage
            .signals_received
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_context_switch(&self, voluntary: bool) {
        let mut usage = self.resource_usage.scheduler.lock();
        if voluntary {
            usage.voluntary_context_switches = usage.voluntary_context_switches.saturating_add(1);
        } else {
            usage.involuntary_context_switches = usage.involuntary_context_switches.saturating_add(1);
        }
    }

    pub(crate) fn parent_pid(&self) -> i32 {
        self.control.lock().parent_pid.unwrap_or(0)
    }

    pub(crate) fn process_group_id(&self) -> i32 {
        self.control.lock().process_group_id
    }

    pub(crate) fn session_id(&self) -> i32 {
        self.control.lock().session_id
    }

    pub(crate) fn scheduler_parameters(&self) -> (i32, i32) {
        let control = self.control.lock();
        (control.scheduling.policy, control.scheduling.priority)
    }

    pub(crate) fn set_scheduler_parameters(&self, policy: i32, priority: i32) {
        let mut control = self.control.lock();
        let sporadic = (control.scheduling.policy == policy)
            .then_some(control.scheduling.sporadic)
            .flatten();
        control.scheduling = SchedulingParameters {
            policy,
            priority,
            sporadic,
        };
    }

    pub(crate) fn scheduling_configuration(&self) -> SchedulingParameters {
        self.control.lock().scheduling
    }

    pub(crate) fn set_scheduling_configuration(&self, scheduling: SchedulingParameters) {
        self.control.lock().scheduling = scheduling;
    }

    pub(crate) fn inherit_spawn_signal_state(
        &self,
        parent: &Process,
        default_signals: SignalSet,
    ) {
        *self.signals.lock() = parent.signals.lock().for_spawn(default_signals);
    }

    pub(crate) fn reset_caught_signal_actions_for_exec(&self) {
        self.signals.lock().reset_caught_actions_for_exec();
    }

    pub(crate) fn nice_priority(&self) -> i32 {
        self.control.lock().nice_priority
    }

    pub(crate) fn set_nice_priority(
        &self,
        caller: &Process,
        priority: i32,
    ) -> Result<(), Error> {
        if !(-20..=19).contains(&priority) {
            return Err(Error::Inval);
        }
        if !self.may_change_scheduling(caller) {
            return Err(Error::Perm);
        }
        let is_root = caller.effective_user_is_root();
        let mut control = self.control.lock();
        if priority < control.nice_priority && !is_root {
            return Err(Error::Perm);
        }
        control.nice_priority = priority;
        Ok(())
    }

    pub(crate) fn resource_limit(&self, resource: i32) -> Result<(u64, u64), Error> {
        let limit = self
            .control
            .lock()
            .resource_limits
            .get(resource)
            .ok_or(Error::Inval)?;
        Ok((limit.soft, limit.hard))
    }

    pub(crate) fn set_resource_limit(
        &self,
        resource: i32,
        soft: u64,
        hard: u64,
    ) -> Result<(), Error> {
        if soft > hard {
            return Err(Error::Inval);
        }
        let is_root = self.effective_user_is_root();
        let mut control = self.control.lock();
        let current = control.resource_limits.get(resource).ok_or(Error::Inval)?;
        if hard > current.hard && !is_root {
            return Err(Error::Perm);
        }
        *control
            .resource_limits
            .get_mut(resource)
            .ok_or(Error::Inval)? = ResourceLimit { soft, hard };
        if resource == 0 {
            self.resource_usage
                .cpu_limit_signals
                .store(0, Ordering::Relaxed);
        }
        Ok(())
    }

    pub(crate) fn check_cpu_limit(&self, cpu_time_ns: u64) {
        let Ok((soft_limit, hard_limit)) = self.resource_limit(0) else {
            return;
        };
        if soft_limit == u64::MAX {
            return;
        }
        let elapsed_seconds = cpu_time_ns / 1_000_000_000;
        if hard_limit != u64::MAX && elapsed_seconds >= hard_limit {
            let _ = self.queue_signal(SignalInfo::new(SIGKILL, 0, 0, 0, 0, 0));
            return;
        }
        let signal_count = self
            .resource_usage
            .cpu_limit_signals
            .load(Ordering::Relaxed);
        let next_threshold = soft_limit.saturating_add(signal_count);
        if elapsed_seconds < next_threshold {
            return;
        }
        if self
            .resource_usage
            .cpu_limit_signals
            .compare_exchange(
                signal_count,
                signal_count.saturating_add(1),
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            let _ = self.queue_signal(SignalInfo::new(SIGXCPU, 0, 0, 0, 0, 0));
        }
    }

    pub(crate) fn signal_action(&self, signal_number: u8) -> Result<SignalAction, Error> {
        self.signals
            .lock()
            .action(signal_number)
            .ok_or(Error::Inval)
    }

    pub(crate) fn set_signal_action(
        &self,
        signal_number: u8,
        action: SignalAction,
    ) -> Result<SignalAction, Error> {
        let (previous, pending) = {
            let mut signals = self.signals.lock();
            let previous = signals.set_action(signal_number, action)?;
            (previous, signals.pending.pending_set())
        };
        ThreadManager::notify_process_signals(self.pid, pending);
        Ok(previous)
    }

    pub(crate) fn queue_signal(&self, information: SignalInfo) -> Result<(), Error> {
        if information.signal_number == 0 || information.signal_number > MAX_SIGNAL_NUMBER {
            return Err(Error::Inval);
        }
        self.record_signal_received();
        if information.signal_number == SIGKILL {
            self.terminate_by_signal(SIGKILL);
            return Ok(());
        }
        if information.signal_number == SIGCONT {
            self.continue_by_signal();
        } else if information.signal_number == SIGSTOP {
            self.stop_by_signal(SIGSTOP);
            return Ok(());
        }
        let pending = {
            let mut signals = self.signals.lock();
            if signals
                .action(information.signal_number)
                .is_some_and(SignalAction::is_ignored)
            {
                return Ok(());
            }
            signals.pending.push(information)?;
            signals.pending.pending_set()
        };
        ThreadManager::notify_process_signals(self.pid, pending);
        Ok(())
    }

    pub(crate) fn pending_signals(&self) -> SignalSet {
        self.signals.lock().pending.pending_set()
    }

    pub(crate) fn take_pending_signal(&self, signals: SignalSet) -> Option<SignalInfo> {
        self.signals.lock().pending.take(signals)
    }

    pub(crate) fn termination_signal(&self) -> Option<u8> {
        self.signals.lock().termination_signal()
    }

    pub(crate) fn terminate_by_signal(&self, signal_number: u8) {
        let first_termination = self
            .signals
            .lock()
            .mark_terminated_by_signal(signal_number);
        if !first_termination {
            return;
        }
        ThreadManager::wake_process_threads(self.pid);
    }

    pub(crate) fn stopped_by_signal(&self) -> Option<u8> {
        self.signals.lock().stopped_by_signal()
    }

    pub(crate) fn stop_by_signal(&self, signal_number: u8) {
        let first_stop = self
            .signals
            .lock()
            .mark_stopped_by_signal(signal_number);
        if first_stop {
            ThreadManager::wake_process_threads(self.pid);
            self.notify_parent(CLD_STOPPED, signal_number as i32);
        }
    }

    pub(crate) fn continue_by_signal(&self) {
        let was_stopped = self.signals.lock().resume();
        if was_stopped {
            ThreadManager::resume_process_threads(self.pid);
            self.notify_parent(CLD_CONTINUED, SIGCONT as i32);
        }
    }

    pub(crate) fn thread_started(&self) {
        self.active_threads.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn thread_exited(&self, status: usize) {
        if self.active_threads.fetch_sub(1, Ordering::AcqRel) == 1
            && !self.exit_notified.swap(true, Ordering::AcqRel)
        {
            crate::kernel::runtime::runtime_libc::filesystem::cleanup_process_descriptors_for_process(
                self.pid,
            );
            if let Some(signal) = self.termination_signal() {
                self.notify_parent(CLD_KILLED, i32::from(signal));
            } else {
                self.notify_parent(CLD_EXITED, status as u8 as i32);
            }
        }
    }

    fn register_child(&self, child: &Process) -> Result<(), Error> {
        let mut children = self.children.lock();
        children.try_reserve(1).map_err(|_| Error::NoMem)?;
        let mut events = Vec::new();
        events
            .try_reserve(CHILD_EVENT_CAPACITY)
            .map_err(|_| Error::NoMem)?;
        children.push(ChildProcessRecord {
            process_id: child.pid,
            process_group_id: child.process_group_id(),
            events,
        });
        Ok(())
    }

    fn record_child_event(&self, status: ChildExitStatus) {
        let mut children = self.children.lock();
        if let Some(child) = children
            .iter_mut()
            .find(|child| child.process_id == status.process_id)
        {
            child.process_group_id = status.process_group_id;
            if child.events.try_reserve(1).is_ok() {
                child.events.push(status);
            } else if matches!(status.code, CLD_EXITED | CLD_KILLED) {
                if let Some(event) = child.events.last_mut() {
                    *event = status;
                }
            }
        }
        drop(children);
        ThreadManager::wake_process_threads(self.pid);
    }

    fn update_child_process_group(&self, process_id: i32, process_group_id: i32) {
        if let Some(child) = self
            .children
            .lock()
            .iter_mut()
            .find(|child| child.process_id == process_id)
        {
            child.process_group_id = process_group_id;
        }
    }

    pub(crate) fn child_wait_status(
        &self,
        process_id: i32,
        wait_exited: bool,
        wait_stopped: bool,
        wait_continued: bool,
        no_wait: bool,
    ) -> Result<Option<ChildExitStatus>, Error> {
        let process_group_id = if process_id == 0 {
            Some(self.process_group_id())
        } else if process_id < -1 {
            Some(process_id.checked_neg().ok_or(Error::Inval)?)
        } else {
            None
        };
        let mut children = self.children.lock();
        let matches_child = |child: &ChildProcessRecord| match process_id {
            value if value > 0 => child.process_id == value,
            -1 => true,
            _ => child.process_group_id == process_group_id.unwrap_or_default(),
        };
        let selected = children
            .iter()
            .enumerate()
            .filter(|(_, child)| matches_child(child))
            .find_map(|(child_index, child)| {
                child
                    .events
                    .iter()
                    .position(|event| match event.code {
                        CLD_EXITED | CLD_KILLED => wait_exited,
                        CLD_STOPPED => wait_stopped,
                        CLD_CONTINUED => wait_continued,
                        _ => false,
                    })
                    .map(|event_index| (child_index, event_index))
            });
        let Some((index, event_index)) = selected else {
            return if children.iter().any(matches_child) {
                Ok(None)
            } else {
                Err(Error::Child)
            };
        };
        let status = children[index].events[event_index];
        if !no_wait {
            if matches!(status.code, CLD_EXITED | CLD_KILLED) {
                self.resource_usage.add_child_usage(status.resource_usage);
                children.remove(index);
            } else {
                children[index].events.remove(event_index);
            }
        }
        Ok(Some(status))
    }

    fn notify_parent(&self, code: i32, status: i32) {
        let parent_pid = self.parent_pid();
        if parent_pid <= 0 {
            return;
        }
        let parent = {
            let registry = PROCESS_REGISTRY.lock();
            Self::lookup_registered(&registry, parent_pid)
        };
        if let Some(parent) = parent {
            parent.record_child_event(ChildExitStatus {
                process_id: self.pid,
                process_group_id: self.process_group_id(),
                code,
                status,
                resource_usage: self.resource_usage.snapshot(),
            });
            if code == CLD_STOPPED
                && parent
                    .signal_action(SIGCHLD)
                    .is_ok_and(|action| action.flags & SA_NOCLDSTOP != 0)
            {
                return;
            }
            let _ = parent.queue_signal(SignalInfo::new(
                SIGCHLD,
                code,
                self.pid,
                self.user_ids()[1],
                0,
                status,
            ));
        }
    }

    pub(crate) fn set_alarm(&self, now_ns: u64, seconds: u32) -> u32 {
        self.signals.lock().set_alarm(now_ns, seconds)
    }

    fn expire_alarm(&self, now_ns: u64) {
        let pending = {
            let mut signals = self.signals.lock();
            if !signals.expire_alarm(now_ns) {
                return;
            }
            signals.pending.pending_set()
        };
        ThreadManager::notify_process_signals(self.pid, pending);
    }

    pub(crate) fn tick_alarms(now_ns: u64) {
        let mut registry = PROCESS_REGISTRY.lock();
        registry.retain(|entry| entry.process.strong_count() != 0);
        for entry in registry.iter() {
            if let Some(process) = entry.process.upgrade() {
                process.expire_alarm(now_ns);
            }
        }
    }

    pub(crate) fn send_signal(
        self: &Arc<Self>,
        target: i32,
        signal_number: i32,
        information: SignalInfo,
    ) -> Result<(), Error> {
        if signal_number < 0 || signal_number as u32 > MAX_SIGNAL_NUMBER as u32 {
            return Err(Error::Inval);
        }
        let target_group = if target == 0 {
            Some(self.process_group_id())
        } else if target < -1 {
            Some(target.checked_neg().ok_or(Error::Inval)?)
        } else {
            None
        };
        let sender_control = *self.control.lock();
        let sender_ids = self.user_ids();
        let mut matched = false;
        let mut permitted = false;
        let mut delivered = false;
        let mut targets = Vec::new();
        {
            let mut registry = PROCESS_REGISTRY.lock();
            registry.retain(|entry| entry.process.strong_count() != 0);
            for entry in registry.iter() {
                let Some(process) = entry.process.upgrade() else {
                    continue;
                };
                let selected = match target {
                    value if value > 0 => process.pid == value,
                    -1 => process.pid != self.pid,
                    _ => {
                        let control = process.control.lock();
                        control.process_group_id == target_group.unwrap_or_default()
                    }
                };
                if !selected {
                    continue;
                }
                matched = true;
                let target_ids = process.user_ids();
                let can_signal = process.pid == self.pid
                    || sender_ids[1] == 0
                    || (signal_number == SIGCONT as i32
                        && sender_control.session_id == process.session_id())
                    || [sender_ids[0], sender_ids[1]]
                        .into_iter()
                        .any(|sender| sender == target_ids[0] || sender == target_ids[2]);
                if !can_signal {
                    if target > 0 {
                        break;
                    }
                    continue;
                }
                permitted = true;
                if signal_number == 0 {
                    delivered = true;
                } else {
                    targets.try_reserve(1).map_err(|_| Error::NoMem)?;
                    targets.push(process);
                }
                if target > 0 {
                    break;
                }
            }
        }
        for process in targets {
            process.queue_signal(SignalInfo::new(
                signal_number as u8,
                information.code,
                self.pid,
                sender_ids[1],
                information.value,
                information.status,
            ))?;
            delivered = true;
        }
        if delivered {
            Ok(())
        } else if matched && !permitted {
            Err(Error::Perm)
        } else {
            Err(Error::Srch)
        }
    }

    pub(crate) fn signal_terminal_process_group(
        session_id: i32,
        process_group_id: i32,
        signal_number: u8,
    ) -> Result<(), Error> {
        if process_group_id <= 0 || signal_number == 0 || signal_number > MAX_SIGNAL_NUMBER {
            return Err(Error::Inval);
        }
        let mut targets = Vec::new();
        {
            let mut registry = PROCESS_REGISTRY.lock();
            registry.retain(|entry| entry.process.strong_count() != 0);
            for entry in registry.iter() {
                let Some(process) = entry.process.upgrade() else {
                    continue;
                };
                let control = *process.control.lock();
                if control.session_id == session_id
                    && control.process_group_id == process_group_id
                {
                    targets.try_reserve(1).map_err(|_| Error::NoMem)?;
                    targets.push(process);
                }
            }
        }
        if targets.is_empty() {
            return Err(Error::Srch);
        }
        let information = SignalInfo::new(signal_number, 0, 0, 0, 0, 0);
        for process in targets {
            process.queue_signal(information)?;
        }
        Ok(())
    }

    pub(crate) fn process_group_for(&self, pid: i32) -> Result<i32, Error> {
        if pid < 0 {
            return Err(Error::Inval);
        }
        if pid == 0 || pid == self.pid {
            return Ok(self.process_group_id());
        }
        let mut registry = PROCESS_REGISTRY.lock();
        registry.retain(|entry| entry.process.strong_count() != 0);
        Self::lookup_registered(&registry, pid)
            .map(|process| process.process_group_id())
            .ok_or(Error::Srch)
    }

    pub(crate) fn session_for(&self, pid: i32) -> Result<i32, Error> {
        if pid < 0 {
            return Err(Error::Inval);
        }
        if pid == 0 || pid == self.pid {
            return Ok(self.session_id());
        }
        let mut registry = PROCESS_REGISTRY.lock();
        registry.retain(|entry| entry.process.strong_count() != 0);
        Self::lookup_registered(&registry, pid)
            .map(|process| process.session_id())
            .ok_or(Error::Srch)
    }

    pub(crate) fn set_process_group(
        self: &Arc<Self>,
        pid: i32,
        process_group_id: i32,
    ) -> Result<(), Error> {
        if pid < 0 || process_group_id < 0 {
            return Err(Error::Inval);
        }
        let mut registry = PROCESS_REGISTRY.lock();
        registry.retain(|entry| entry.process.strong_count() != 0);
        let target = if pid == 0 || pid == self.pid {
            Arc::clone(self)
        } else {
            Self::lookup_registered(&registry, pid).ok_or(Error::Srch)?
        };
        let caller_control = *self.control.lock();
        let target_snapshot = *target.control.lock();
        if target.pid != self.pid && target_snapshot.parent_pid != Some(self.pid) {
            return Err(Error::Perm);
        }
        if target_snapshot.session_id != caller_control.session_id
            || target_snapshot.session_id == target.pid
        {
            return Err(Error::Perm);
        }
        let process_group_id = if process_group_id == 0 {
            target.pid
        } else {
            process_group_id
        };
        if process_group_id != target.pid
            && !Self::process_group_exists(
                &registry,
                process_group_id,
                target_snapshot.session_id,
            )
        {
            return Err(Error::Perm);
        }
        let mut target_control = target.control.lock();
        if target_control.session_id != target_snapshot.session_id
            || (target.pid != self.pid && target_control.parent_pid != Some(self.pid))
        {
            return Err(Error::Srch);
        }
        target_control.process_group_id = process_group_id;
        drop(target_control);
        if let Some(parent_pid) = target_snapshot.parent_pid {
            if let Some(parent) = Self::lookup_registered(&registry, parent_pid) {
                parent.update_child_process_group(target.pid, process_group_id);
            }
        }
        Ok(())
    }

    pub(crate) fn create_session(&self) -> Result<i32, Error> {
        let mut registry = PROCESS_REGISTRY.lock();
        registry.retain(|entry| entry.process.strong_count() != 0);
        if registry.iter().any(|entry| {
                entry.process.upgrade().is_some_and(|process| {
                    process.control.lock().process_group_id == self.pid
                })
            })
        {
            return Err(Error::Perm);
        }
        let mut control = self.control.lock();
        control.session_id = self.pid;
        control.process_group_id = self.pid;
        control.controlling_terminal = false;
        self.pty_terminal.lock().take();
        Ok(self.pid)
    }

    pub(crate) fn controlling_terminal_session(&self) -> Result<i32, Error> {
        let _registry = PROCESS_REGISTRY.lock();
        let control = *self.control.lock();
        if control.controlling_terminal
            && crate::kernel::tty::session_owns_console(control.session_id)
        {
            return Ok(control.session_id);
        }
        if self
            .pty_terminal
            .lock()
            .as_ref()
            .is_some_and(|terminal| terminal.session_owns(control.session_id))
        {
            return Ok(control.session_id);
        }
        Err(Error::NotTty)
    }

    pub(crate) fn acquire_console_terminal(&self, no_controlling_terminal: bool) {
        if no_controlling_terminal {
            return;
        }
        let _registry = PROCESS_REGISTRY.lock();
        let mut control = self.control.lock();
        if control.controlling_terminal
            || control.session_id != self.pid
            || control.process_group_id != self.pid
            || self.pty_terminal.lock().is_some()
        {
            return;
        }
        if crate::kernel::tty::acquire_session(control.session_id, control.process_group_id) {
            control.controlling_terminal = true;
        }
    }

    pub(crate) fn acquire_pty_terminal(
        &self,
        terminal: Arc<crate::kernel::tty::Pty>,
        no_controlling_terminal: bool,
    ) {
        if no_controlling_terminal {
            return;
        }
        let _registry = PROCESS_REGISTRY.lock();
        let control = *self.control.lock();
        if control.controlling_terminal
            || control.session_id != self.pid
            || control.process_group_id != self.pid
            || self.pty_terminal.lock().is_some()
        {
            return;
        }
        if terminal.acquire_session(control.session_id, control.process_group_id) {
            *self.pty_terminal.lock() = Some(terminal);
        }
    }

    pub(crate) fn controlling_pty_terminal(
        &self,
    ) -> Option<Arc<crate::kernel::tty::Pty>> {
        let session_id = self.control.lock().session_id;
        self.pty_terminal
            .lock()
            .as_ref()
            .filter(|terminal| terminal.session_owns(session_id))
            .map(Arc::clone)
    }

    pub(crate) fn terminal_foreground_process_group(&self) -> Result<i32, Error> {
        let _registry = PROCESS_REGISTRY.lock();
        let control = *self.control.lock();
        if control.controlling_terminal
            && crate::kernel::tty::session_owns_console(control.session_id)
        {
            return crate::kernel::tty::foreground_process_group(control.session_id)
                .ok_or(Error::NotTty);
        }
        self.pty_terminal
            .lock()
            .as_ref()
            .filter(|terminal| terminal.session_owns(control.session_id))
            .and_then(|terminal| terminal.foreground_process_group(control.session_id))
            .ok_or(Error::NotTty)
    }

    pub(crate) fn set_terminal_foreground_process_group(
        &self,
        process_group_id: i32,
    ) -> Result<(), Error> {
        if process_group_id <= 0 {
            return Err(Error::Inval);
        }
        let control = *self.control.lock();
        let session_id = control.session_id;
        let pty_terminal = self.pty_terminal.lock().as_ref().map(Arc::clone);
        let console_terminal = control.controlling_terminal
            && crate::kernel::tty::session_owns_console(session_id);
        let owns_pty = pty_terminal
            .as_ref()
            .is_some_and(|terminal| terminal.session_owns(session_id));
        if !console_terminal && !owns_pty {
            return Err(Error::NotTty);
        }
        let foreground_process_group = if console_terminal {
            crate::kernel::tty::foreground_process_group(session_id)
        } else {
            pty_terminal
                .as_ref()
                .and_then(|terminal| terminal.foreground_process_group(session_id))
        };
        if foreground_process_group.is_some_and(|foreground| {
            foreground != control.process_group_id
        }) {
            Self::signal_terminal_process_group(
                session_id,
                control.process_group_id,
                crate::kernel::signal::SIGTTOU,
            )?;
            return Err(Error::Intr);
        }
        let mut registry = PROCESS_REGISTRY.lock();
        registry.retain(|entry| entry.process.strong_count() != 0);
        if !Self::process_group_exists(&registry, process_group_id, session_id) {
            return Err(if Self::process_group_exists_any(&registry, process_group_id) {
                Error::Perm
            } else {
                Error::Srch
            });
        }
        let updated = if console_terminal {
            crate::kernel::tty::set_foreground_process_group(session_id, process_group_id)
        } else {
            pty_terminal.as_ref().is_some_and(|terminal| {
                terminal.set_foreground_process_group(session_id, process_group_id)
            })
        };
        if !updated {
            return Err(Error::NotTty);
        }
        Ok(())
    }

    pub(crate) fn id(self: &Arc<Self>) -> u64 {
        Arc::as_ptr(self) as usize as u64
    }

    pub(crate) fn namespace_set(&self) -> NamespaceSet {
        self.namespace_set.clone()
    }

    pub(crate) fn address_space(&self) -> UserAddressSpace {
        self.address_space.lock().clone()
    }

    pub(crate) fn replace_address_space(&self, address_space: UserAddressSpace) {
        *self.address_space.lock() = address_space;
    }

    pub(crate) fn discard_unstarted_child(&self, process_id: i32) {
        self.children
            .lock()
            .retain(|child| child.process_id != process_id);
        PROCESS_REGISTRY.lock().retain(|entry| entry.pid != process_id);
    }

    pub(crate) fn current_directory(&self) -> Result<Vec<u8>, Error> {
        self.filesystem.lock().try_clone().map(|context| context.current_directory)
    }

    pub(crate) fn set_current_directory(&self, path: &[u8]) -> Result<(), Error> {
        if path.first() != Some(&b'/') || path.contains(&0) {
            return Err(Error::Inval);
        }
        let mut current_directory = Vec::new();
        current_directory
            .try_reserve_exact(path.len())
            .map_err(|_| Error::NoMem)?;
        current_directory.extend_from_slice(path);
        self.filesystem.lock().current_directory = current_directory;
        Ok(())
    }

    pub(crate) fn umask(&self) -> u32 {
        self.filesystem.lock().umask
    }

    pub(crate) fn set_umask(&self, mask: u32) -> u32 {
        let mut filesystem = self.filesystem.lock();
        let previous = filesystem.umask;
        filesystem.umask = mask & 0o777;
        previous
    }

    pub(crate) fn credentials(&self) -> Result<Credentials, Error> {
        self.credentials.lock().try_clone()
    }

    pub(crate) fn user_ids(&self) -> [u32; 3] {
        let credentials = self.credentials.lock();
        [
            credentials.real_user.raw(),
            credentials.effective_user.raw(),
            credentials.saved_user.raw(),
        ]
    }

    pub(crate) fn group_ids(&self) -> [u32; 3] {
        let credentials = self.credentials.lock();
        [
            credentials.real_group.raw(),
            credentials.effective_group.raw(),
            credentials.saved_group.raw(),
        ]
    }

    pub(crate) fn supplementary_group_ids(&self) -> Result<Vec<u32>, Error> {
        let credentials = self.credentials.lock();
        let mut groups = Vec::new();
        groups
            .try_reserve_exact(credentials.supplementary_groups.len())
            .map_err(|_| Error::NoMem)?;
        groups.extend(
            credentials
                .supplementary_groups()
                .iter()
                .map(|group| group.raw()),
        );
        Ok(groups)
    }

    pub(crate) fn effective_user_is_root(&self) -> bool {
        self.credentials.lock().is_root()
    }

    pub(crate) fn set_user_id(&self, user: u32) -> Result<(), Error> {
        self.credentials.lock().set_user_id(UserId::new(user))
    }

    pub(crate) fn set_effective_user(&self, user: u32) -> Result<(), Error> {
        self.credentials
            .lock()
            .set_effective_user(UserId::new(user))
    }

    pub(crate) fn set_real_and_effective_user(
        &self,
        real_user: Option<u32>,
        effective_user: Option<u32>,
    ) -> Result<(), Error> {
        self.credentials.lock().set_real_and_effective_user(
            real_user.map(UserId::new),
            effective_user.map(UserId::new),
        )
    }

    pub(crate) fn set_all_user_ids(
        &self,
        real_user: Option<u32>,
        effective_user: Option<u32>,
        saved_user: Option<u32>,
    ) -> Result<(), Error> {
        self.credentials.lock().set_all_user_ids(
            real_user.map(UserId::new),
            effective_user.map(UserId::new),
            saved_user.map(UserId::new),
        )
    }

    pub(crate) fn set_group_id(&self, group: u32) -> Result<(), Error> {
        self.credentials.lock().set_group_id(GroupId::new(group))
    }

    pub(crate) fn set_effective_group(&self, group: u32) -> Result<(), Error> {
        self.credentials
            .lock()
            .set_effective_group(GroupId::new(group))
    }

    pub(crate) fn set_real_and_effective_group(
        &self,
        real_group: Option<u32>,
        effective_group: Option<u32>,
    ) -> Result<(), Error> {
        self.credentials.lock().set_real_and_effective_group(
            real_group.map(GroupId::new),
            effective_group.map(GroupId::new),
        )
    }

    pub(crate) fn set_all_group_ids(
        &self,
        real_group: Option<u32>,
        effective_group: Option<u32>,
        saved_group: Option<u32>,
    ) -> Result<(), Error> {
        self.credentials.lock().set_all_group_ids(
            real_group.map(GroupId::new),
            effective_group.map(GroupId::new),
            saved_group.map(GroupId::new),
        )
    }

    pub(crate) fn set_supplementary_group_ids(&self, groups: &[u32]) -> Result<(), Error> {
        if groups.len() > MAX_SUPPLEMENTARY_GROUPS {
            return Err(Error::Inval);
        }
        let mut copied_groups = Vec::new();
        copied_groups
            .try_reserve_exact(groups.len())
            .map_err(|_| Error::NoMem)?;
        copied_groups.extend(groups.iter().copied().map(GroupId::new));
        self.credentials
            .lock()
            .set_supplementary_groups(copied_groups)
    }

    pub(crate) fn insert_file_object(&self, file: FileHandle) -> Result<KernelObjectRef, Error> {
        self.kernel_objects.lock().insert(file)
    }

    pub(crate) fn insert_file_descriptor(&self, file: FileHandle) -> Result<u32, Error> {
        let object_ref = self.insert_file_object(file)?;
        let descriptor = self.file_descriptors.lock().insert(object_ref);
        match descriptor {
            Ok(descriptor) => Ok(descriptor),
            Err(error) => {
                let _ = self.close_kernel_object(object_ref);
                Err(error)
            }
        }
    }

    pub(crate) fn file_descriptor(&self, descriptor: u32) -> Result<Arc<KernelObject>, Error> {
        let object_ref = self.file_descriptors.lock().get(descriptor)?;
        self.kernel_object(object_ref)
    }

    pub(crate) fn close_file_descriptor(&self, descriptor: u32) -> Result<(), Error> {
        let object_ref = self.file_descriptors.lock().remove(descriptor)?;
        self.close_kernel_object(object_ref)
    }

    pub(crate) fn kernel_object(
        &self,
        reference: KernelObjectRef,
    ) -> Result<Arc<KernelObject>, Error> {
        self.kernel_objects.lock().get(reference)
    }

    pub(crate) fn close_kernel_object(&self, reference: KernelObjectRef) -> Result<(), Error> {
        let object = self.kernel_objects.lock().remove(reference)?;
        drop(object);
        Ok(())
    }

    pub(crate) fn insert_capability(&self, capability: Capability) -> Result<CapabilityHandle, Error> {
        self.capabilities.lock().insert(capability)
    }

    pub(crate) fn create_endpoint_pair(
        &self,
        capacity: usize,
    ) -> Result<(CapabilityHandle, CapabilityHandle), Error> {
        let (sender, receiver) = Endpoint::channel(capacity)?;
        let mut table = self.capabilities.lock();
        let sender = table.insert(Capability::Endpoint(sender))?;
        match table.insert(Capability::Endpoint(receiver)) {
            Ok(receiver) => Ok((sender, receiver)),
            Err(error) => {
                let _ = table.remove(sender);
                Err(error)
            }
        }
    }

    pub(crate) fn capability(&self, handle: CapabilityHandle) -> Result<Capability, Error> {
        self.capabilities.lock().get(handle)?.duplicate()
    }

    pub(crate) fn restrict_capability(
        &self,
        handle: CapabilityHandle,
        rights: CapabilityRights,
    ) -> Result<CapabilityHandle, Error> {
        let capability = self.capabilities.lock().get(handle)?.restrict(rights)?;
        self.capabilities.lock().insert(capability)
    }

    pub(crate) fn prepare_transfer(
        &self,
        handle: CapabilityHandle,
        rights: CapabilityRights,
    ) -> Result<Capability, Error> {
        self.capabilities
            .lock()
            .get(handle)?
            .prepare_transfer(rights)
    }

    pub(crate) fn attach_capability(
        &self,
        body: &mut MessageBody,
        handle: CapabilityHandle,
        rights: CapabilityRights,
    ) -> Result<(), Error> {
        let capability = self.prepare_transfer(handle, rights)?;
        body.push_capability(capability)
    }

    pub(crate) fn try_send(
        &self,
        handle: CapabilityHandle,
        body: MessageBody,
    ) -> Result<(), Error> {
        match self.capability(handle)? {
            Capability::Endpoint(endpoint) => endpoint.try_send(body),
            Capability::SharedMemory(_) => Err(Error::Badf),
        }
    }

    pub(crate) fn send(
        &self,
        handle: CapabilityHandle,
        body: MessageBody,
    ) -> Result<SendFuture, Error> {
        match self.capability(handle)? {
            Capability::Endpoint(endpoint) => Ok(endpoint.send(body)),
            Capability::SharedMemory(_) => Err(Error::Badf),
        }
    }

    pub(crate) fn call_now(
        &self,
        handle: CapabilityHandle,
        body: MessageBody,
    ) -> Result<CallTicket, Error> {
        match self.capability(handle)? {
            Capability::Endpoint(endpoint) => endpoint.call_now(body),
            Capability::SharedMemory(_) => Err(Error::Badf),
        }
    }

    pub(crate) fn call(
        &self,
        handle: CapabilityHandle,
        body: MessageBody,
    ) -> Result<CallFuture, Error> {
        match self.capability(handle)? {
            Capability::Endpoint(endpoint) => endpoint.call(body),
            Capability::SharedMemory(_) => Err(Error::Badf),
        }
    }

    pub(crate) fn map_shared_memory(
        &self,
        handle: CapabilityHandle,
    ) -> Result<SharedMemoryMapping, Error> {
        match self.capability(handle)? {
            Capability::SharedMemory(grant) => grant.map_user(self.address_space()),
            Capability::Endpoint(_) => Err(Error::Badf),
        }
    }

    pub(crate) fn try_receive(
        &self,
        handle: CapabilityHandle,
    ) -> Result<ProcessMessage, Error> {
        let endpoint = self.receive_endpoint(handle)?;
        let received = endpoint.try_receive()?;
        self.into_process_message(received)
    }

    pub(crate) async fn receive(
        &self,
        handle: CapabilityHandle,
    ) -> Result<ProcessMessage, Error> {
        let endpoint = self.receive_endpoint(handle)?;
        let received = endpoint.receive().await?;
        self.into_process_message(received)
    }

    fn receive_endpoint(&self, handle: CapabilityHandle) -> Result<Endpoint, Error> {
        match self.capability(handle)? {
            Capability::Endpoint(endpoint) => Ok(endpoint),
            Capability::SharedMemory(_) => Err(Error::Badf),
        }
    }

    fn into_process_message(&self, received: ReceivedMessage) -> Result<ProcessMessage, Error> {
        let (body, reply) = received.into_parts();
        let (opcode, payload, capabilities) = body.into_parts();
        let mut handles = Vec::new();
        handles
            .try_reserve_exact(capabilities.len())
            .map_err(|_| Error::NoMem)?;
        for capability in capabilities {
            match self.insert_capability(capability) {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    for handle in handles {
                        let _ = self.close_capability(handle);
                    }
                    return Err(error);
                }
            }
        }
        Ok(ProcessMessage {
            opcode,
            payload,
            capabilities: handles,
            reply,
        })
    }

    pub(crate) fn close_capability(&self, handle: CapabilityHandle) -> Result<(), Error> {
        drop(self.capabilities.lock().remove(handle)?);
        Ok(())
    }
}

pub(crate) struct ProcessMessage {
    opcode: u32,
    payload: Vec<u8>,
    capabilities: Vec<CapabilityHandle>,
    reply: Option<ReplyCapability>,
}

impl ProcessMessage {
    pub(crate) const fn opcode(&self) -> u32 {
        self.opcode
    }

    pub(crate) fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub(crate) fn capabilities(&self) -> &[CapabilityHandle] {
        &self.capabilities
    }

    pub(crate) fn take_capability(&mut self, index: usize) -> Option<CapabilityHandle> {
        (index < self.capabilities.len()).then(|| self.capabilities.remove(index))
    }

    pub(crate) fn reply(self, response: Result<MessageBody, Error>) -> Result<(), Error> {
        self.reply.ok_or(Error::Inval)?.reply(response)
    }
}

#[cfg(test)]
#[path = "../tests/kernel/process.rs"]
mod tests;