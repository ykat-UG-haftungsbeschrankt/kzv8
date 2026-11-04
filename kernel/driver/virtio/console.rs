/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::fmt::Write as _;

use crate::kernel::driver::virtio::serial::VirtioSerial;
use crate::kernel::driver::virtio::transport::VirtioTransport;

pub(crate) struct Console;

struct ConsoleWriter;

impl core::fmt::Write for ConsoleWriter {
    fn write_str(&mut self, message: &str) -> core::fmt::Result {
        if VirtioSerial::write(message.as_bytes()) {
            Ok(())
        } else {
            Err(core::fmt::Error)
        }
    }
}

impl Console {
    pub(crate) fn initialize() -> bool {
        VirtioSerial::initialize()
    }

    pub(crate) fn write_device_list() {
        let mut writer = ConsoleWriter;
        crate::kernel::driver::virtio::transport::for_each_device(|device| {
            let _ = core::write!(
                &mut writer,
                "kz: virtio device_id={} type={}\n",
                device.device_id,
                VirtioTransport::device_type_name(device.device_id)
            );
        });
    }

    pub(crate) fn write(message: &[u8]) -> bool {
        VirtioSerial::write(message)
    }

    pub(crate) fn handle_interrupt(pending: u64) {
        VirtioSerial::handle_interrupt(pending)
    }
}
