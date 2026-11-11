/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::kernel::error::Error;
use crate::kernel::ipc::{
    CallFuture, CallTicket, Capability, CapabilityRights, Endpoint, MessageBody, ReceivedMessage,
    ReplyCapability, SendFuture, SharedMemoryMapping,
};
use crate::kernel::lib::hashmap::HashMap;
use crate::kernel::namespace::NamespaceSet;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::scheduler::thread::vm::UserAddressSpace;
use crate::kernel::vfs::FileHandle;

const INITIAL_CAPABILITY_SLOTS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UserId(u32);

impl UserId {
    pub(crate) const ROOT: Self = Self(0);

    pub(crate) const fn new(id: u32) -> Self {
        Self(id)
    }

    pub(crate) const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GroupId(u32);

impl GroupId {
    pub(crate) const ROOT: Self = Self(0);

    pub(crate) const fn new(id: u32) -> Self {
        Self(id)
    }

    pub(crate) const fn raw(self) -> u32 {
        self.0
    }
}

pub(crate) struct Credentials {
    real_user: UserId,
    effective_user: UserId,
    real_group: GroupId,
    effective_group: GroupId,
    supplementary_groups: Vec<GroupId>,
}

impl Credentials {
    pub(crate) const fn root() -> Self {
        Self {
            real_user: UserId::ROOT,
            effective_user: UserId::ROOT,
            real_group: GroupId::ROOT,
            effective_group: GroupId::ROOT,
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
            real_group,
            effective_group,
            supplementary_groups,
        }
    }

    pub(crate) const fn real_user(&self) -> UserId {
        self.real_user
    }

    pub(crate) const fn effective_user(&self) -> UserId {
        self.effective_user
    }

    pub(crate) const fn real_group(&self) -> GroupId {
        self.real_group
    }

    pub(crate) const fn effective_group(&self) -> GroupId {
        self.effective_group
    }

    pub(crate) fn is_member_of(&self, group: GroupId) -> bool {
        self.effective_group == group || self.supplementary_groups.contains(&group)
    }

    pub(crate) const fn is_root(&self) -> bool {
        self.effective_user.0 == 0
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
            real_group: self.real_group,
            effective_group: self.effective_group,
            supplementary_groups,
        })
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
}

impl CapabilityTable {
    fn new() -> Result<Self, Error> {
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(INITIAL_CAPABILITY_SLOTS)
            .map_err(|_| Error::NoMem)?;
        slots.resize_with(INITIAL_CAPABILITY_SLOTS, CapabilitySlot::empty);
        Ok(Self { slots })
    }

    fn insert(&mut self, capability: Capability) -> Result<CapabilityHandle, Error> {
        let index = if let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.capability.is_none())
        {
            index
        } else {
            self.slots.try_reserve(1).map_err(|_| Error::NoMem)?;
            let index = self.slots.len();
            self.slots.push(CapabilitySlot::empty());
            index
        };
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
        slot.capability.take().ok_or(Error::Badf)
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

    fn from_object(object: &Arc<KernelObject>) -> Self {
        Self(Arc::as_ptr(object) as usize as u64)
    }
}

pub(crate) struct KernelObject {
    file: IrqSpinLock<FileHandle>,
}

impl KernelObject {
    fn new(file: FileHandle) -> Self {
        Self {
            file: IrqSpinLock::new(file),
        }
    }

    pub(crate) fn file(&self) -> &IrqSpinLock<FileHandle> {
        &self.file
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
        let reference = KernelObjectRef::from_object(&object);
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

pub(crate) struct Process {
    namespace_set: NamespaceSet,
    address_space: UserAddressSpace,
    credentials: IrqSpinLock<Credentials>,
    capabilities: IrqSpinLock<CapabilityTable>,
    kernel_objects: IrqSpinLock<KernelObjectTable>,
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
        let address_space = UserAddressSpace::new()?;
        let capabilities = CapabilityTable::new()?;
        Ok(Arc::new(Self {
            namespace_set,
            address_space,
            credentials: IrqSpinLock::new(credentials),
            capabilities: IrqSpinLock::new(capabilities),
            kernel_objects: IrqSpinLock::new(KernelObjectTable::new()),
        }))
    }

    pub(crate) fn id(self: &Arc<Self>) -> u64 {
        Arc::as_ptr(self) as usize as u64
    }

    pub(crate) fn namespace_set(&self) -> NamespaceSet {
        self.namespace_set.clone()
    }

    pub(crate) fn address_space(&self) -> UserAddressSpace {
        self.address_space.clone()
    }

    pub(crate) fn credentials(&self) -> Result<Credentials, Error> {
        self.credentials.lock().try_clone()
    }

    pub(crate) fn insert_file_object(&self, file: FileHandle) -> Result<KernelObjectRef, Error> {
        self.kernel_objects.lock().insert(file)
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
            Capability::Endpoint(endpoint) => Ok(endpoint.call(body)),
            Capability::SharedMemory(_) => Err(Error::Badf),
        }
    }

    pub(crate) fn map_shared_memory(
        &self,
        handle: CapabilityHandle,
    ) -> Result<SharedMemoryMapping, Error> {
        match self.capability(handle)? {
            Capability::SharedMemory(grant) => grant.map_user(self.address_space.clone()),
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
mod tests {
    use alloc::sync::Arc;

    use super::{Capability, CapabilityTable, Process};
    use crate::kernel::ipc::Endpoint;

    #[test]
    fn capability_table_grows_past_previous_limit() {
        let (endpoint, _receiver) = Endpoint::channel(1).unwrap();
        let mut table = CapabilityTable::new().unwrap();
        for _ in 0..4100 {
            table
                .insert(Capability::Endpoint(endpoint.clone()))
                .unwrap();
        }
        assert_eq!(table.slots.len(), 4100);
    }

    #[test]
    fn process_identity_is_its_arc_allocation_address() {
        let process = Process::new().unwrap();
        assert_eq!(process.id(), Arc::as_ptr(&process) as usize as u64);
    }
}