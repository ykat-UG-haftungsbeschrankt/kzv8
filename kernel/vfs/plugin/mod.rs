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

use super::{DirectoryEntry, DirectoryPlugin, FilePlugin, Node, VfsFuture, VfsResult};

struct Directory {
    entries: Vec<DirectoryEntry>,
}

impl DirectoryPlugin for Directory {
    fn lookup<'a>(&'a self, name: &'a str) -> VfsFuture<'a, VfsResult<Node>> {
        Box::pin(async move {
            self.entries
                .iter()
                .find(|entry| entry.name.as_str() == name)
                .map(|entry| entry.node.clone())
                .ok_or(Error::NoEnt)
        })
    }

    fn read_dir(&self) -> VfsFuture<'_, VfsResult<Vec<DirectoryEntry>>> {
        Box::pin(async move {
            Ok(self
                .entries
                .iter()
                .map(|entry| DirectoryEntry {
                    name: entry.name.clone(),
                    node: entry.node.clone(),
                })
                .collect())
        })
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
}

fn directory(entries: Vec<DirectoryEntry>) -> Node {
    Node::directory(Directory { entries })
}

fn named_directory(entries: Vec<(&'static str, Node)>) -> Node {
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

pub(crate) fn build_virtio_tree(virtio_fs_root: Option<Node>) -> (Node, usize) {
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
    let devices = named_directory(vec![
        ("console", directory(console_devices)),
        ("net", directory(network_devices)),
        ("block", directory(block_devices)),
        ("fs", directory(file_system_devices)),
    ]);
    let mut root_entries = vec![("dev", devices)];
    if let Some(filesystem) = virtio_fs_root {
        root_entries.push(("fs", filesystem));
    }
    (named_directory(root_entries), device_count)
}
