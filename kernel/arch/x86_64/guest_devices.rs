/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::arch::traits::DebugArchitecture as _;

pub(crate) struct PortIoExit {
    pub(crate) input: bool,
    pub(crate) port: u16,
    pub(crate) width: u8,
    pub(crate) value: u32,
}

pub(crate) fn emulate_port_io(exit: PortIoExit) -> Option<u32> {
    match (exit.input, exit.port, exit.width) {
        (false, 0x00e9 | 0x03f8, 1) => {
            crate::kernel::arch::DebugArchitecture::decommit_debug(exit.value as u8);
            Some(0)
        }
        (true, 0x00e9 | 0x03f8, 1) => Some(0xff),
        (true, 0x03f9, 1) => Some(0),
        (true, 0x03fa, 1) => Some(0),
        (true, 0x03fb, 1) => Some(0),
        (true, 0x03fc, 1) => Some(0),
        (true, 0x03fd, 1) => Some(0x60),
        (true, 0x03fe, 1) => Some(0xb0),
        _ => None,
    }
}