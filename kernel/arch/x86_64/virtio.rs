/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::arch::VirtioWindow;

pub(crate) const fn virtio_window() -> VirtioWindow {
    VirtioWindow {
        start: 0xfeb0_0000,
        slots: 24,
        stride: 0x200,
    }
}