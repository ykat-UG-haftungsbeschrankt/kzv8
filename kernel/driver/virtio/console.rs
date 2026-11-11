/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::driver::virtio::serial::VirtioSerial;

pub(crate) struct Console;

impl Console {
    pub(crate) fn initialize() -> bool {
        VirtioSerial::initialize()
    }

    pub(crate) fn write(message: &[u8]) -> bool {
        VirtioSerial::write(message)
    }

    pub(crate) fn read(output: &mut [u8]) -> usize {
        VirtioSerial::read(output)
    }

    pub(crate) fn handle_interrupt(pending: u64) {
        VirtioSerial::handle_interrupt(pending)
    }
}
