/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::kernel::scheduler::sync::IrqSpinLock;

static NEXT_NAMESPACE_ID: AtomicU64 = AtomicU64::new(1);
static ROOT_NAMESPACE_SET: IrqSpinLock<Option<NamespaceSet>> = IrqSpinLock::new(None);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// A namespace category whose identity can be compared between process sets.
///
/// These tags do not currently scope or isolate resources in their named subsystem.
pub(crate) enum NamespaceKind {
    Pid,
    Mount,
    Network,
    Ipc,
    Uts,
    User,
}

struct Namespace {
    id: u64,
}

impl Namespace {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            id: NEXT_NAMESPACE_ID.fetch_add(1, Ordering::Relaxed),
        })
    }
}

#[derive(Clone)]
/// Namespace identity scaffolding carried by a process.
///
/// This type records namespace identities and sharing relationships only. Creating a
/// distinct set does not isolate PID reporting, VFS or mount lookup, networking, IPC,
/// UTS data, or user-namespace mappings. Subsystems remain shared unless they enforce
/// isolation independently; `NamespaceSet` is not a security boundary.
pub(crate) struct NamespaceSet {
    pid: Arc<Namespace>,
    mount: Arc<Namespace>,
    network: Arc<Namespace>,
    ipc: Arc<Namespace>,
    uts: Arc<Namespace>,
    user: Arc<Namespace>,
}

impl NamespaceSet {
    fn new_isolated() -> Self {
        Self {
            pid: Namespace::new(),
            mount: Namespace::new(),
            network: Namespace::new(),
            ipc: Namespace::new(),
            uts: Namespace::new(),
            user: Namespace::new(),
        }
    }

    pub(crate) fn root() -> Self {
        ROOT_NAMESPACE_SET
            .lock()
            .get_or_insert_with(Self::new_isolated)
            .clone()
    }

    pub(crate) fn new_container() -> Self {
        Self::new_isolated()
    }

    fn namespace(&self, kind: NamespaceKind) -> &Arc<Namespace> {
        match kind {
            NamespaceKind::Pid => &self.pid,
            NamespaceKind::Mount => &self.mount,
            NamespaceKind::Network => &self.network,
            NamespaceKind::Ipc => &self.ipc,
            NamespaceKind::Uts => &self.uts,
            NamespaceKind::User => &self.user,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn id(&self, kind: NamespaceKind) -> u64 {
        self.namespace(kind).id
    }

    #[allow(dead_code)]
    pub(crate) fn shares(&self, other: &Self, kind: NamespaceKind) -> bool {
        Arc::ptr_eq(self.namespace(kind), other.namespace(kind))
    }
}
