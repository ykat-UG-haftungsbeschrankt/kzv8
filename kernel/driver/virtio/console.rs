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

    pub(crate) fn is_readable() -> bool {
        VirtioSerial::input_ready()
    }

    pub(crate) fn is_available() -> bool {
        VirtioSerial::is_available()
    }

    pub(crate) fn output_pending() -> bool {
        VirtioSerial::output_pending()
    }

    pub(crate) fn is_writable() -> bool {
        VirtioSerial::output_ready()
    }

    pub(crate) fn flush_input() {
        VirtioSerial::flush_input();
    }

    pub(crate) fn flush_queued_output() {
        VirtioSerial::flush_queued_output();
    }

    pub(crate) fn handle_interrupt(pending: u64) {
        VirtioSerial::handle_interrupt(pending)
    }
}
