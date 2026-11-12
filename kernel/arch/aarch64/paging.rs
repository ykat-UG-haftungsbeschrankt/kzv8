/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ptr::{addr_of_mut, write_volatile};

const TABLE_ENTRIES: usize = 512;
const BLOCK_DESCRIPTOR: u64 = 0b01;
const ACCESS_FLAG: u64 = 1 << 10;
const INNER_SHAREABLE: u64 = 0b11 << 8;
const DEVICE_ATTRIBUTE: u64 = 1 << 2;
const PXN: u64 = 1 << 53;
const UXN: u64 = 1 << 54;

const DEVICE_WINDOW: u64 = BLOCK_DESCRIPTOR | ACCESS_FLAG | DEVICE_ATTRIBUTE | PXN | UXN;
const RAM_WINDOW: u64 = 0x4000_0000 | BLOCK_DESCRIPTOR | ACCESS_FLAG | INNER_SHAREABLE | UXN;

const MAIR_EL1_VALUE: u64 = 0x44;
const TCR_EL1_VALUE: u64 = 25
    | (1 << 8)
    | (1 << 10)
    | (0b11 << 12)
    | (25 << 16)
    | (1 << 23)
    | (1 << 24)
    | (1 << 26)
    | (0b11 << 28)
    | (0b10 << 30)
    | (0b10 << 32);

#[repr(C, align(4096))]
pub(super) struct Level1Table(pub(super) [u64; TABLE_ENTRIES]);

pub(super) static mut TTBR0_TABLE: Level1Table = Level1Table([0; TABLE_ENTRIES]);

pub(crate) unsafe fn initialize() {
    let table = addr_of_mut!(TTBR0_TABLE.0).cast::<u64>();
    write_volatile(table, DEVICE_WINDOW);
    write_volatile(table.add(1), RAM_WINDOW);

    unsafe { initialize_translation_table(table as u64) };
}

pub(crate) unsafe fn initialize_secondary() {
    let table = addr_of_mut!(TTBR0_TABLE.0).cast::<u64>();
    unsafe { initialize_translation_table(table as u64) };
}

unsafe fn initialize_translation_table(table: u64) {
    core::arch::asm!(
        "dsb sy",
        "msr mair_el1, {mair}",
        "msr tcr_el1, {tcr}",
        "msr ttbr0_el1, {ttbr}",
        "dsb sy",
        "tlbi vmalle1",
        "dsb sy",
        "isb",
        "mrs {sctlr}, sctlr_el1",
        "bic {sctlr}, {sctlr}, #4",
        "bic {sctlr}, {sctlr}, #4096",
        "orr {sctlr}, {sctlr}, #1",
        "msr sctlr_el1, {sctlr}",
        "isb",
        mair = in(reg) MAIR_EL1_VALUE,
        tcr = in(reg) TCR_EL1_VALUE,
        ttbr = in(reg) table,
        sctlr = lateout(reg) _,
        options(nostack, preserves_flags),
    );
}
