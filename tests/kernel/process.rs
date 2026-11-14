/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
    use crate::kernel::error::Error;
    use alloc::sync::Arc;
    use alloc::vec::Vec;
    use core::sync::atomic::{AtomicBool, AtomicUsize};

    use super::{
        Capability, CapabilityTable, Credentials, DEFAULT_RESOURCE_LIMITS, FileDescriptorTable,
        FilesystemContext, GroupId, KernelObjectRef, KernelObjectTable, NamespaceSet, Process,
        ProcessControlState, ProcessResourceUsage, SchedulingParameters, SignalState,
        UserAddressSpace, UserId,
    };
    use crate::kernel::ipc::{Endpoint, EndpointApi as _};
    use crate::kernel::scheduler::sync::IrqSpinLock;
    use crate::kernel::signal::{
        SignalInfo, SignalSet, CLD_CONTINUED, CLD_KILLED, CLD_STOPPED, SIGCHLD, SIGCONT, SIGKILL,
        SIGSTOP,
    };
    use crate::kernel::vfs::{
        DirectoryHandle, DirectoryHandleReference, NodeMetadata, NodeMetadataApi as _,
    };

    struct TestDirectoryReference {
        path: Vec<u8>,
        drop_count: Arc<AtomicUsize>,
    }

    impl DirectoryHandleReference for TestDirectoryReference {
        fn path(&self) -> Result<Vec<u8>, Error> {
            let mut path = Vec::new();
            path.try_reserve_exact(self.path.len())
                .map_err(|_| Error::NoMem)?;
            path.extend_from_slice(&self.path);
            Ok(path)
        }
    }

    impl Drop for TestDirectoryReference {
        fn drop(&mut self) {
            self.drop_count
                .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
    }


impl Process {
    fn new_test_process(
        parent: Option<&Arc<Self>>,
        credentials: Credentials,
    ) -> Result<Arc<Self>, Error> {
        if parent.is_none() {
            crate::kernel::tty::tests::reset_for_test();
        }
        let pid = Self::allocate_pid()?;
        let control = parent.map_or(
            ProcessControlState {
                parent_pid: None,
                process_group_id: pid,
                session_id: pid,
                controlling_terminal: false,
                scheduling: SchedulingParameters::OTHER,
                nice_priority: 0,
                resource_limits: DEFAULT_RESOURCE_LIMITS,
            },
            |parent| ProcessControlState {
                parent_pid: Some(parent.pid),
                ..*parent.control.lock()
            },
        );
        let filesystem = match parent {
            Some(parent) => parent.filesystem.lock().try_clone()?,
            None => FilesystemContext::root()?,
        };
        let process = Arc::new(Self {
            pid,
            active_threads: AtomicUsize::new(0),
            resource_usage: ProcessResourceUsage::new(),
            exit_notified: AtomicBool::new(false),
            control: IrqSpinLock::new(control),
            pty_terminal: IrqSpinLock::new(None),
            namespace_set: parent.map_or_else(NamespaceSet::root, |parent| parent.namespace_set()),
            address_space: IrqSpinLock::new(UserAddressSpace::new_test_stub()),
            signals: IrqSpinLock::new(SignalState::new()),
            credentials: IrqSpinLock::new(credentials),
            capabilities: IrqSpinLock::new(CapabilityTable::new()?),
            kernel_objects: IrqSpinLock::new(KernelObjectTable::new()),
            file_descriptors: IrqSpinLock::new(FileDescriptorTable::new()),
            children: IrqSpinLock::new(Vec::new()),
            filesystem: IrqSpinLock::new(filesystem),
        });
        Self::register(&process)?;
        Ok(process)
    }
}

    static PROCESS_CONTROL_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn cloned_filesystem_context_retains_directory_handle() {
        let drop_count = Arc::new(AtomicUsize::new(0));
        let filesystem = FilesystemContext {
            current_directory: DirectoryHandle::new(TestDirectoryReference {
                path: b"/work".to_vec(),
                drop_count: Arc::clone(&drop_count),
            }),
            umask: 0o022,
        };
        let child = filesystem.try_clone().unwrap();

        drop(filesystem);
        assert_eq!(drop_count.load(core::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(child.current_directory.path().unwrap(), b"/work");

        drop(child);
        assert_eq!(drop_count.load(core::sync::atomic::Ordering::Relaxed), 1);
    }

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
    fn capability_table_reuses_removed_slot_with_new_generation() {
        let (endpoint, _receiver) = Endpoint::channel(1).unwrap();
        let mut table = CapabilityTable::new().unwrap();
        let first = table
            .insert(Capability::Endpoint(endpoint.clone()))
            .unwrap();
        let removed = table.remove(first).unwrap();
        drop(removed);
        let second = table.insert(Capability::Endpoint(endpoint)).unwrap();
        assert_eq!(first.raw() as u32, second.raw() as u32);
        assert_ne!(first, second);
    }

    #[test]
    fn process_identity_is_its_arc_allocation_address() {
        let process = Arc::new(Process {
            pid: 1,
            active_threads: AtomicUsize::new(0),
            resource_usage: ProcessResourceUsage::new(),
            exit_notified: AtomicBool::new(false),
            control: IrqSpinLock::new(ProcessControlState {
                parent_pid: None,
                process_group_id: 1,
                session_id: 1,
                controlling_terminal: false,
                scheduling: SchedulingParameters::OTHER,
                nice_priority: 0,
                resource_limits: DEFAULT_RESOURCE_LIMITS,
            }),
            pty_terminal: IrqSpinLock::new(None),
            namespace_set: NamespaceSet::root(),
            address_space: IrqSpinLock::new(UserAddressSpace::new_test_stub()),
            signals: IrqSpinLock::new(SignalState::new()),
            credentials: IrqSpinLock::new(Credentials::root()),
            capabilities: IrqSpinLock::new(CapabilityTable::new().unwrap()),
            kernel_objects: IrqSpinLock::new(KernelObjectTable::new()),
            file_descriptors: IrqSpinLock::new(FileDescriptorTable::new()),
            children: IrqSpinLock::new(Vec::new()),
            filesystem: IrqSpinLock::new(FilesystemContext::root().unwrap()),
        });
        assert_eq!(process.id(), Arc::as_ptr(&process) as usize as u64);
    }

    #[test]
    fn file_descriptors_start_after_stdio_and_reuse_closed_slots() {
        let mut descriptors = FileDescriptorTable::new();
        let first = KernelObjectRef::from_raw(0x1000);
        let second = KernelObjectRef::from_raw(0x2000);
        let third = KernelObjectRef::from_raw(0x3000);

        assert_eq!(descriptors.insert(first).unwrap(), 3);
        assert_eq!(descriptors.insert(second).unwrap(), 4);
        assert_eq!(descriptors.get(4).unwrap(), second);
        assert_eq!(descriptors.remove(3).unwrap(), first);
        assert_eq!(descriptors.insert(third).unwrap(), 3);
        assert_eq!(descriptors.get(3).unwrap(), third);
        assert!(descriptors.get(0).is_err());
    }

    #[test]
    fn credential_changes_validate_saved_ids_and_drop_root_privilege() {
        let _guard = PROCESS_CONTROL_TEST_LOCK.lock().unwrap();
        let mut saved_id_rules = Credentials::root();
        saved_id_rules
            .set_all_user_ids(
                Some(UserId::new(1000)),
                Some(UserId::new(2000)),
                Some(UserId::new(3000)),
            )
            .unwrap();
        saved_id_rules
            .set_real_and_effective_user(None, None)
            .unwrap();
        assert_eq!(saved_id_rules.saved_user.raw(), 3000);
        saved_id_rules
            .set_real_and_effective_user(Some(UserId::new(1000)), None)
            .unwrap();
        assert_eq!(saved_id_rules.saved_user.raw(), 2000);
        saved_id_rules
            .set_real_and_effective_user(None, Some(UserId::new(2000)))
            .unwrap();
        assert_eq!(saved_id_rules.saved_user.raw(), 2000);
        saved_id_rules
            .set_real_and_effective_user(Some(UserId::new(1000)), None)
            .unwrap();
        assert_eq!(saved_id_rules.saved_user.raw(), 2000);

        let mut distinct_user_ids = Credentials::root();
        distinct_user_ids
            .set_all_user_ids(
                Some(UserId::new(1000)),
                Some(UserId::new(2000)),
                Some(UserId::new(3000)),
            )
            .unwrap();
        assert_eq!(distinct_user_ids.set_effective_user(UserId::new(2000)), Ok(()));
        distinct_user_ids
            .set_real_and_effective_user(None, Some(UserId::new(2000)))
            .unwrap();

        let mut saved_group_rules = Credentials::root();
        saved_group_rules
            .set_all_group_ids(
                Some(GroupId::new(100)),
                Some(GroupId::new(200)),
                Some(GroupId::new(300)),
            )
            .unwrap();
        saved_group_rules
            .set_real_and_effective_group(None, None)
            .unwrap();
        assert_eq!(saved_group_rules.saved_group.raw(), 300);
        saved_group_rules
            .set_real_and_effective_group(Some(GroupId::new(100)), None)
            .unwrap();
        assert_eq!(saved_group_rules.saved_group.raw(), 200);
        saved_group_rules
            .set_real_and_effective_group(None, Some(GroupId::new(200)))
            .unwrap();
        assert_eq!(saved_group_rules.saved_group.raw(), 200);
        saved_group_rules
            .set_real_and_effective_group(Some(GroupId::new(100)), None)
            .unwrap();
        assert_eq!(saved_group_rules.saved_group.raw(), 200);

        let mut distinct_group_ids = Credentials::root();
        distinct_group_ids
            .set_all_group_ids(
                Some(GroupId::new(100)),
                Some(GroupId::new(200)),
                Some(GroupId::new(300)),
            )
            .unwrap();
        assert_eq!(distinct_group_ids.set_effective_group(GroupId::new(200)), Ok(()));
        distinct_group_ids
            .set_real_and_effective_group(None, Some(GroupId::new(200)))
            .unwrap();

        let mut credentials = Credentials::root();
        credentials.set_user_id(UserId::new(1000)).unwrap();
        assert_eq!(credentials.real_user.raw(), 1000);
        assert_eq!(credentials.effective_user.raw(), 1000);
        assert_eq!(credentials.saved_user.raw(), 1000);
        assert_eq!(credentials.set_user_id(UserId::ROOT), Err(crate::kernel::error::Error::Perm));

        let mut unprivileged = Credentials::new(
            UserId::new(1000),
            UserId::new(2000),
            GroupId::new(100),
            GroupId::new(200),
            Vec::new(),
        );
        assert_eq!(unprivileged.set_effective_user(UserId::new(2000)), Ok(()));
        assert_eq!(
            unprivileged.set_effective_user(UserId::new(3000)),
            Err(crate::kernel::error::Error::Perm)
        );
        assert_eq!(
            unprivileged.set_group_id(GroupId::new(300)),
            Err(crate::kernel::error::Error::Perm)
        );
        assert_eq!(
            unprivileged.set_supplementary_groups(Vec::new()),
            Err(crate::kernel::error::Error::Perm)
        );

        let process = Process::new_test_process(None, Credentials::root()).unwrap();
        process.set_supplementary_group_ids(&[7, 9]).unwrap();
        assert_eq!(process.supplementary_group_ids().unwrap(), [7, 9]);
        process.set_user_id(1000).unwrap();
        assert_eq!(process.user_ids(), [1000, 1000, 1000]);
        assert_eq!(process.set_user_id(0), Err(crate::kernel::error::Error::Perm));
        assert_eq!(
            process.set_supplementary_group_ids(&[1]),
            Err(crate::kernel::error::Error::Perm)
        );

        let mut maximum_uid = Credentials::root();
        maximum_uid.set_user_id(UserId::new(u32::MAX)).unwrap();
        assert_eq!(maximum_uid.effective_user.raw(), u32::MAX);
    }

    #[test]
    fn node_access_uses_effective_and_supplementary_groups() {
        let metadata = NodeMetadata::with_owner_and_mode(
            UserId::new(1000),
            GroupId::new(100),
            0o640,
        )
        .unwrap();
        let owner = Credentials::new(
            UserId::new(1000),
            UserId::new(1000),
            GroupId::new(200),
            GroupId::new(200),
            Vec::new(),
        );
        assert_eq!(metadata.check_access(&owner, 0o6, false), Ok(()));

        let unrelated = Credentials::new(
            UserId::new(2000),
            UserId::new(2000),
            GroupId::new(200),
            GroupId::new(200),
            Vec::new(),
        );
        assert_eq!(
            metadata.check_access(&unrelated, 0o4, false),
            Err(crate::kernel::error::Error::Acces)
        );

        let group_member = Credentials::new(
            UserId::new(2000),
            UserId::new(2000),
            GroupId::new(200),
            GroupId::new(200),
            alloc::vec![GroupId::new(100)],
        );
        assert_eq!(metadata.check_access(&group_member, 0o4, false), Ok(()));
        assert_eq!(
            metadata.check_access(&group_member, 0o2, false),
            Err(crate::kernel::error::Error::Acces)
        );
    }

    #[test]
    fn process_group_session_and_controlling_terminal_transitions_are_validated() {
        let _guard = PROCESS_CONTROL_TEST_LOCK.lock().unwrap();
        let parent = Process::new_test_process(None, Credentials::root()).unwrap();
        let child = Process::new_test_process(
            Some(&parent),
            Credentials::new(
                UserId::new(1000),
                UserId::new(1000),
                GroupId::new(100),
                GroupId::new(100),
                Vec::new(),
            ),
        )
        .unwrap();
        assert!(parent.pid() > 0);
        assert!(child.pid() > parent.pid());
        assert_eq!(child.parent_pid(), parent.pid());
        assert_eq!(parent.process_group_for(child.pid()), Ok(parent.pid()));
        assert_eq!(child.session_for(parent.pid()), Ok(parent.session_id()));
        assert_eq!(parent.controlling_terminal_session(), Ok(parent.session_id()));
        assert_eq!(child.set_user_id(2000), Err(crate::kernel::error::Error::Perm));
        assert_eq!(child.set_user_id(1000), Ok(()));

        parent.set_process_group(child.pid(), 0).unwrap();
        assert_eq!(child.process_group_id(), child.pid());
        assert_eq!(child.create_session(), Err(crate::kernel::error::Error::Perm));
        assert_eq!(
            child.set_terminal_foreground_process_group(child.pid()),
            Err(Error::Intr)
        );
        parent
            .set_terminal_foreground_process_group(child.pid())
            .unwrap();
        assert_eq!(parent.terminal_foreground_process_group(), Ok(child.pid()));

        let session_child =
            Process::new_test_process(Some(&parent), parent.credentials().unwrap()).unwrap();
        assert_eq!(session_child.create_session(), Ok(session_child.pid()));
        assert_eq!(session_child.session_id(), session_child.pid());
        assert_eq!(
            session_child.controlling_terminal_session(),
            Err(crate::kernel::error::Error::NotTty)
        );
        assert_eq!(
            parent.set_process_group(session_child.pid(), 0),
            Err(crate::kernel::error::Error::Perm)
        );
        assert_eq!(
            session_child.set_process_group(parent.pid(), 0),
            Err(crate::kernel::error::Error::Perm)
        );
        child
            .set_terminal_foreground_process_group(parent.process_group_id())
            .unwrap();
        assert_eq!(
            parent.set_terminal_foreground_process_group(session_child.pid()),
            Err(crate::kernel::error::Error::Perm)
        );
    }

    #[test]
    fn process_signal_delivery_notifies_parent_of_child_state_changes() {
        let _guard = PROCESS_CONTROL_TEST_LOCK.lock().unwrap();
        let parent = Process::new_test_process(None, Credentials::root()).unwrap();
        let child =
            Process::new_test_process(Some(&parent), Credentials::root()).unwrap();

        parent
            .send_signal(
                child.pid(),
                10,
                SignalInfo::new(10, 0, parent.pid(), 0, 0x55, 0),
            )
            .unwrap();
        assert_eq!(
            child
                .take_pending_signal(SignalSet::from_signal(10))
                .unwrap()
                .value,
            0x55
        );

        child
            .queue_signal(SignalInfo::new(SIGSTOP, 0, parent.pid(), 0, 0, 0))
            .unwrap();
        assert_eq!(child.stopped_by_signal(), Some(SIGSTOP));
        assert_eq!(
            parent
                .take_pending_signal(SignalSet::from_signal(SIGCHLD))
                .unwrap()
                .code,
            CLD_STOPPED
        );

        child
            .queue_signal(SignalInfo::new(SIGCONT, 0, parent.pid(), 0, 0, 0))
            .unwrap();
        assert_eq!(child.stopped_by_signal(), None);
        assert_eq!(
            parent
                .take_pending_signal(SignalSet::from_signal(SIGCHLD))
                .unwrap()
                .code,
            CLD_CONTINUED
        );

        child.thread_started();
        child
            .queue_signal(SignalInfo::new(SIGKILL, 0, parent.pid(), 0, 0, 0))
            .unwrap();
        assert_eq!(child.termination_signal(), Some(SIGKILL));
        child.thread_exited(0);
        let killed = parent
            .take_pending_signal(SignalSet::from_signal(SIGCHLD))
            .unwrap();
        assert_eq!(killed.code, CLD_KILLED);
        assert_eq!(killed.status, SIGKILL as i32);
    }
