/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::kernel::error::Error;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::vfs::{FileSystem, MountOptions, Node, VfsFuture};

pub(crate) struct TestFileSystem {
    root: Node,
    sync_count: Arc<AtomicUsize>,
    unmount_count: Arc<AtomicUsize>,
    mount_options: Arc<IrqSpinLock<Vec<(String, String)>>>,
}

impl FileSystem for TestFileSystem {
    fn root(&self) -> Result<Node, Error> {
        Ok(self.root.clone())
    }

    fn mount_root<'a>(&'a self, options: MountOptions<'a>) -> VfsFuture<'a, Result<Node, Error>> {
        let result = (|| {
            let mut stored_options = Vec::new();
            stored_options
                .try_reserve_exact(options.len())
                .map_err(|_| Error::NoMem)?;
            for (key, value) in options {
                let mut key_copy = String::new();
                key_copy
                    .try_reserve_exact(key.len())
                    .map_err(|_| Error::NoMem)?;
                key_copy.push_str(key);
                let mut value_copy = String::new();
                value_copy
                    .try_reserve_exact(value.len())
                    .map_err(|_| Error::NoMem)?;
                value_copy.push_str(value);
                stored_options.push((key_copy, value_copy));
            }
            *self.mount_options.lock() = stored_options;
            Ok(self.root.clone())
        })();
        Box::pin(async move { result })
    }

    fn sync(&self) -> VfsFuture<'_, Result<(), Error>> {
        self.sync_count.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }

    fn unmount(&self) -> VfsFuture<'_, Result<(), Error>> {
        self.unmount_count.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
}

pub(crate) fn test_filesystem(
    root: Node,
) -> (
    Arc<dyn FileSystem>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    Arc<IrqSpinLock<Vec<(String, String)>>>,
) {
    let sync_count = Arc::new(AtomicUsize::new(0));
    let unmount_count = Arc::new(AtomicUsize::new(0));
    let mount_options = Arc::new(IrqSpinLock::new(Vec::new()));
    let filesystem: Arc<dyn FileSystem> = Arc::new(TestFileSystem {
        root,
        sync_count: Arc::clone(&sync_count),
        unmount_count: Arc::clone(&unmount_count),
        mount_options: Arc::clone(&mount_options),
    });
    (filesystem, sync_count, unmount_count, mount_options)
}