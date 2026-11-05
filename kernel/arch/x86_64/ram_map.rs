/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ptr::{addr_of_mut, read_volatile, write_volatile};

const TABLE_ENTRIES: usize = 512;
const FIRST_HIGH_PDPT_ENTRY: usize = 4;
const MAX_PDPT_ENTRIES: usize = 64;
pub(crate) const IDENTITY_MAP_LIMIT: usize = MAX_PDPT_ENTRIES * TABLE_ENTRIES * (1 << 21);
const PRESENT_WRITABLE: u64 = 0x3;
const LARGE_PAGE: u64 = 1 << 7;
const PAGE_ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
struct PageDirectory([u64; TABLE_ENTRIES]);

static mut HIGH_DIRECTORIES: [PageDirectory; MAX_PDPT_ENTRIES - FIRST_HIGH_PDPT_ENTRY] =
    [PageDirectory([0; TABLE_ENTRIES]); MAX_PDPT_ENTRIES - FIRST_HIGH_PDPT_ENTRY];

pub(crate) fn is_identity_mapped(address: usize) -> bool {
    address < IDENTITY_MAP_LIMIT
}

extern "C" {
    static mut pml4: u64;
}

pub(crate) unsafe fn map_high_memory() {
    let pml4_table = addr_of_mut!(pml4).cast::<u64>();
    let pml4_entry = unsafe { read_volatile(pml4_table) };
    if pml4_entry & PRESENT_WRITABLE == 0 {
        return;
    }
    let pdpt = (pml4_entry & PAGE_ADDRESS_MASK) as *mut u64;
    let directories = addr_of_mut!(HIGH_DIRECTORIES).cast::<PageDirectory>();

    for pdpt_index in FIRST_HIGH_PDPT_ENTRY..MAX_PDPT_ENTRIES {
        let directory = unsafe { directories.add(pdpt_index - FIRST_HIGH_PDPT_ENTRY) };
        let entries = unsafe { addr_of_mut!((*directory).0).cast::<u64>() };
        for directory_index in 0..TABLE_ENTRIES {
            let frame_index = pdpt_index * TABLE_ENTRIES + directory_index;
            let address = (frame_index as u64) << 21;
            unsafe {
                write_volatile(
                    entries.add(directory_index),
                    address | PRESENT_WRITABLE | LARGE_PAGE,
                )
            };
        }
        let directory_address = directory as u64 & PAGE_ADDRESS_MASK;
        unsafe {
            write_volatile(pdpt.add(pdpt_index), directory_address | PRESENT_WRITABLE);
        }
    }

    let cr3: u64;
    unsafe {
        core::arch::asm!(
            "mov {value}, cr3",
            value = out(reg) cr3,
            options(nomem, nostack, preserves_flags),
        );
        core::arch::asm!(
            "mov cr3, {value}",
            value = in(reg) cr3,
            options(nomem, nostack, preserves_flags),
        );
    }
}
