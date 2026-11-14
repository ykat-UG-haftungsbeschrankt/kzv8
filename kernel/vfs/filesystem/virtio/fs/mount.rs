/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;

use crate::kernel::vfs::{FileSystem, Node, VfsResult};

use super::VirtioFs;

struct MountedVirtioFs {
    filesystem: Arc<VirtioFs>,
}

impl FileSystem for MountedVirtioFs {
    fn root(&self) -> VfsResult<Node> {
        Ok(self.filesystem.root())
    }
}

pub(crate) fn mountable(filesystem: &Arc<VirtioFs>) -> Arc<dyn FileSystem> {
    Arc::new(MountedVirtioFs {
        filesystem: Arc::clone(filesystem),
    })
}