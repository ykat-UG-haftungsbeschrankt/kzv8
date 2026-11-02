/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) fn debugcon(message: &[u8]) {
    for byte in message {
        unsafe {
            core::arch::asm!(
                "out dx, al",
                in("dx") 0xe9u16,
                in("al") *byte,
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}
