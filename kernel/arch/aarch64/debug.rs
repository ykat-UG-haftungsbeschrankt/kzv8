/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::sync::atomic::{AtomicU8, Ordering};

const UART_BASE: usize = 0x0900_0000;
const UART_DATA: usize = 0x00;
const UART_FLAGS: usize = 0x18;
const UART_INTEGER_BAUD_DIVISOR: usize = 0x24;
const UART_FRACTIONAL_BAUD_DIVISOR: usize = 0x28;
const UART_LINE_CONTROL: usize = 0x2c;
const UART_CONTROL: usize = 0x30;
const UART_TX_FIFO_FULL: u32 = 1 << 5;
const UART_INITIALIZING: u8 = 1;
const UART_INITIALIZED: u8 = 2;
const UART_POLL_LIMIT: usize = 100_000;

static UART_STATE: AtomicU8 = AtomicU8::new(0);

pub(crate) fn write_early(bytes: &[u8]) {
    initialize_uart();
    let mut previous = 0;
    for byte in bytes.iter().copied() {
        if byte == b'\n' && previous != b'\r' {
            write_byte(b'\r');
        }
        write_byte(byte);
        previous = byte;
    }
}

fn initialize_uart() {
    loop {
        match UART_STATE.compare_exchange(
            0,
            UART_INITIALIZING,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                write_register(UART_CONTROL, 0);
                write_register(UART_INTEGER_BAUD_DIVISOR, 13);
                write_register(UART_FRACTIONAL_BAUD_DIVISOR, 1);
                write_register(UART_LINE_CONTROL, 0x70);
                write_register(UART_CONTROL, 0x301);
                UART_STATE.store(UART_INITIALIZED, Ordering::Release);
                return;
            }
            Err(UART_INITIALIZED) => return,
            Err(_) => core::hint::spin_loop(),
        }
    }
}

fn write_byte(byte: u8) {
    for _ in 0..UART_POLL_LIMIT {
        if read_register(UART_FLAGS) & UART_TX_FIFO_FULL == 0 {
            write_register(UART_DATA, u32::from(byte));
            return;
        }
        core::hint::spin_loop();
    }
}

fn write_register(offset: usize, value: u32) {
    let register = (UART_BASE + offset) as *mut u32;
    // SAFETY: This AArch64 target is QEMU virt, whose PL011 is at this aligned MMIO address.
    unsafe { core::ptr::write_volatile(register, value) };
}

fn read_register(offset: usize) -> u32 {
    let register = (UART_BASE + offset) as *const u32;
    // SAFETY: This AArch64 target is QEMU virt, whose PL011 is at this aligned MMIO address.
    unsafe { core::ptr::read_volatile(register) }
}