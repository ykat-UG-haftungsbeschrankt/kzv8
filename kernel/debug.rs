/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::fmt::{self, Write as _};

use crate::kernel::arch::traits::DebugArchitecture as _;

pub(crate) fn write(bytes: &[u8]) {
    crate::kernel::arch::DebugArchitecture::write_early(bytes);
}

pub(crate) fn debug(message: &str) {
    write(message.as_bytes());
}

pub(crate) fn debug_fmt(arguments: fmt::Arguments<'_>) {
    let _ = DebugWriter.write_fmt(arguments);
}

pub(crate) fn write_virtio_devices() {
    crate::kernel::driver::virtio::transport::for_each_device(|device| {
        debug_fmt(format_args!(
            "kz: virtio device_id={} type={}\n",
            device.device_id,
            crate::kernel::driver::virtio::transport::VirtioTransport::device_type_name(
                device.device_id,
            )
        ));
    });
}

struct DebugWriter;

impl fmt::Write for DebugWriter {
    fn write_str(&mut self, message: &str) -> fmt::Result {
        debug(message);
        Ok(())
    }
}