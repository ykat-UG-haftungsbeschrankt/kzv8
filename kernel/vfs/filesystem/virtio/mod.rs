/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) mod queue;
pub(crate) mod service;
pub(crate) mod transport;

#[allow(dead_code)]
pub(crate) mod balloon;
pub(crate) mod block;
pub(crate) mod console;
#[allow(dead_code)]
pub(crate) mod fs;
#[allow(dead_code)]
pub(crate) mod input;
pub(crate) mod net;
#[allow(dead_code)]
pub(crate) mod rtc;
#[allow(dead_code)]
pub(crate) mod scsi;
pub(crate) mod serial;
#[allow(dead_code)]
pub(crate) mod vga;

mod devices;

#[allow(unused_imports)]
pub(crate) use devices::VirtioDeviceFileSystem;
