/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::kernel::driver::virtio::transport::{self, VirtioDeviceInfo, VirtioTransport};
use crate::kernel::error::Error;
use crate::kernel::lib::hashmap::HashMap;
use crate::kernel::scheduler::sync::IrqSpinLock;

use super::tun;
use super::super::{
    DirectoryEntry, DirectoryPlugin, FilePlugin, Node, NodeApi as _, VfsFuture, VfsResult,
};

struct Directory {
    //@FIXME: INDEXTYPE(entries); hash-indexed child names provide expected O(1) lookup.
    entries: IrqSpinLock<HashMap<String, Node, 8>>,
}

impl DirectoryPlugin for Directory {
    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, VfsResult<Node>> {
        let result = self.entries.lock().get(name).cloned().ok_or(Error::NoEnt);
        Box::pin(async move { result })
    }

    fn read_dir(&self) -> VfsFuture<'_, VfsResult<Vec<DirectoryEntry>>> {
        let result = (|| {
            let mut entries = Vec::new();
            let directory = self.entries.lock();
            entries
                .try_reserve_exact(directory.len())
                .map_err(|_| Error::NoMem)?;
            directory.try_for_each(|name, node| {
                let mut name_copy = String::new();
                name_copy
                    .try_reserve_exact(name.len())
                    .map_err(|_| Error::NoMem)?;
                name_copy.push_str(name);
                entries.push(DirectoryEntry {
                    name: name_copy,
                    node: node.clone(),
                });
                Ok::<(), Error>(())
            })?;
            Ok(entries)
        })();
        Box::pin(async move { result })
    }
}

struct DeviceMetadataFile {
    contents: String,
}

impl DeviceMetadataFile {
    fn new(device: VirtioDeviceInfo) -> Self {
        Self {
            contents: format!(
                "device_id={}\ntype={}\nslot={}\n",
                device.device_id,
                VirtioTransport::device_type_name(device.device_id),
                device.slot
            ),
        }
    }
}

impl FilePlugin for DeviceMetadataFile {
    fn read_at<'a>(&'a self, offset: u64, buffer: &'a mut [u8]) -> VfsFuture<'a, VfsResult<usize>> {
        Box::pin(async move {
            let start = usize::try_from(offset).map_err(|_| Error::Inval)?;
            if start >= self.contents.len() {
                return Ok(0);
            }
            let length = buffer.len().min(self.contents.len() - start);
            buffer[..length].copy_from_slice(&self.contents.as_bytes()[start..start + length]);
            Ok(length)
        })
    }

    fn size(&self) -> VfsFuture<'_, VfsResult<u64>> {
        let length = self.contents.len() as u64;
        Box::pin(async move { Ok(length) })
    }
}

fn directory(entries: Vec<DirectoryEntry>) -> Result<Node, Error> {
    let mut directory = HashMap::new();
    for entry in entries {
        match directory.insert(entry.name, entry.node) {
            Ok(None) => {}
            Ok(Some(_previous)) => return Err(Error::Exist),
            Err((_name, _node)) => return Err(Error::NoMem),
        }
    }
    Ok(Node::directory(Directory {
        entries: IrqSpinLock::new(directory),
    }))
}

fn named_directory(entries: Vec<(&'static str, Node)>) -> Result<Node, Error> {
    directory(
        entries
            .into_iter()
            .map(|(name, node)| DirectoryEntry {
                name: String::from(name),
                node,
            })
            .collect(),
    )
}

pub(crate) fn build_virtio_tree(virtio_fs_root: Option<Node>) -> Result<(Node, usize), Error> {
    let mut block_devices = Vec::new();
    let mut network_devices = Vec::new();
    let mut console_devices = Vec::new();
    let mut file_system_devices = Vec::new();

    transport::for_each_device(|device| {
        let (prefix, entries) = match device.device_id {
            1 => ("eth", &mut network_devices),
            2 => ("vd", &mut block_devices),
            3 => ("tty", &mut console_devices),
            26 => ("virtiofs", &mut file_system_devices),
            _ => return,
        };
        let ordinal = entries.len() + 1;
        entries.push(DirectoryEntry {
            name: format!("{}{}", prefix, ordinal),
            node: Node::file(DeviceMetadataFile::new(device)),
        });
    });

    let device_count = block_devices.len()
        + network_devices.len()
        + console_devices.len()
        + file_system_devices.len();
    network_devices.push(DirectoryEntry {
        name: String::from("tun"),
        node: Node::file(tun::TunDevicePlugin),
    });
    let devices = named_directory(vec![
        ("console", directory(console_devices)?),
        ("net", directory(network_devices)?),
        ("block", directory(block_devices)?),
        ("fs", directory(file_system_devices)?),
    ])?;
    let mut root_entries = vec![("dev", devices)];
    if let Some(filesystem) = virtio_fs_root {
        root_entries.push(("fs", filesystem));
    }
    Ok((named_directory(root_entries)?, device_count))
}

#[cfg(test)]
#[path = "../../../tests/kernel/vfs/plugin/plugin.rs"]
mod tests;