/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::vfs::{FileSystem, Node, VfsResult};

pub(crate) struct VirtioDeviceFileSystem;

impl FileSystem for VirtioDeviceFileSystem {
    fn root(&self) -> VfsResult<Node> {
        crate::kernel::vfs::plugin::build_virtio_tree(None).map(|(root, _device_count)| root)
    }
}