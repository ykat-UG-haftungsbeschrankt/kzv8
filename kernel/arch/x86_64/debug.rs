/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::sync::atomic::{AtomicU8, Ordering};

const COM1_BASE: u16 = 0x3f8;
const COM1_INTERRUPT_ENABLE: u16 = COM1_BASE + 1;
const COM1_FIFO_CONTROL: u16 = COM1_BASE + 2;
const COM1_LINE_CONTROL: u16 = COM1_BASE + 3;
const COM1_MODEM_CONTROL: u16 = COM1_BASE + 4;
const COM1_LINE_STATUS: u16 = COM1_BASE + 5;
const COM1_TRANSMIT_EMPTY: u8 = 1 << 5;
const SERIAL_INITIALIZING: u8 = 1;
const SERIAL_INITIALIZED: u8 = 2;
const SERIAL_POLL_LIMIT: usize = 100_000;

static SERIAL_STATE: AtomicU8 = AtomicU8::new(0);

pub(crate) fn write_early(bytes: &[u8]) {
    initialize_com1();
    for byte in bytes {
        write_byte(*byte);
    }
}

fn initialize_com1() {
    loop {
        match SERIAL_STATE.compare_exchange(
            0,
            SERIAL_INITIALIZING,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                write_port(COM1_INTERRUPT_ENABLE, 0);
                write_port(COM1_LINE_CONTROL, 0x80);
                write_port(COM1_BASE, 1);
                write_port(COM1_INTERRUPT_ENABLE, 0);
                write_port(COM1_LINE_CONTROL, 0x03);
                write_port(COM1_FIFO_CONTROL, 0xc7);
                write_port(COM1_MODEM_CONTROL, 0x0b);
                SERIAL_STATE.store(SERIAL_INITIALIZED, Ordering::Release);
                return;
            }
            Err(SERIAL_INITIALIZED) => return,
            Err(_) => core::hint::spin_loop(),
        }
    }
}

fn write_byte(byte: u8) {
    for _ in 0..SERIAL_POLL_LIMIT {
        if read_port(COM1_LINE_STATUS) & COM1_TRANSMIT_EMPTY != 0 {
            write_port(COM1_BASE, byte);
            return;
        }
        core::hint::spin_loop();
    }
}

fn write_port(port: u16, value: u8) {
    // SAFETY: The kernel runs at ring 0 and COM1 is the x86 early serial port.
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

fn read_port(port: u16) -> u8 {
    let value: u8;
    // SAFETY: The kernel runs at ring 0 and COM1 is the x86 early serial port.
    unsafe {
        core::arch::asm!(
            "in al, dx",
            in("dx") port,
            out("al") value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

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
