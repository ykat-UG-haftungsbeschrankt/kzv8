/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::mem::size_of;
use core::ptr::{addr_of, addr_of_mut, write_volatile};

use crate::kernel::scheduler::sync::IrqSpinLock;

const HOST_TSS_SELECTOR: u16 = 0x18;
const CODE_SEGMENT: u64 = 0x00af_9a00_0000_ffff;
const DATA_SEGMENT: u64 = 0x00cf_9200_0000_ffff;
const USER_DATA_SEGMENT: u64 = 0x00cf_f200_0000_ffff;
const USER_CODE_SEGMENT: u64 = 0x00af_fa00_0000_ffff;
const IA32_GS_BASE: u32 = 0xc000_0101;
const IA32_KERNEL_GS_BASE: u32 = 0xc000_0102;

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct TaskStateSegment {
    reserved_0: u32,
    rsp: [u64; 3],
    reserved_1: u64,
    ist: [u64; 7],
    reserved_2: u64,
    reserved_3: u16,
    io_map_base: u16,
}

impl TaskStateSegment {
    const fn new() -> Self {
        Self {
            reserved_0: 0,
            rsp: [0; 3],
            reserved_1: 0,
            ist: [0; 7],
            reserved_2: 0,
            reserved_3: 0,
            io_map_base: size_of::<Self>() as u16,
        }
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct HostSegmentSlot {
    kernel_rsp: u64,
    user_rsp: u64,
    apic_id: u32,
    initialized: bool,
    gdt: [u64; 7],
    tss: TaskStateSegment,
}

impl HostSegmentSlot {
    const fn new() -> Self {
        Self {
            kernel_rsp: 0,
            user_rsp: 0,
            apic_id: u32::MAX,
            initialized: false,
            gdt: [
                0,
                CODE_SEGMENT,
                DATA_SEGMENT,
                0,
                0,
                USER_DATA_SEGMENT,
                USER_CODE_SEGMENT,
            ],
            tss: TaskStateSegment::new(),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct HostSegmentState {
    pub(crate) gdt_base: u64,
    pub(crate) tss_base: u64,
    pub(crate) tss_selector: u16,
}

//@FIXME: INDEXTYPE(HOST_SEGMENTS); stable boxed slots with linear APIC-ID lookup; add a HashMap side index for expected O(1) lookup if this grows.
static HOST_SEGMENTS: IrqSpinLock<Vec<Box<HostSegmentSlot>>> =
    IrqSpinLock::new(Vec::new());

pub(crate) fn initialize() -> Option<HostSegmentState> {
    let apic_id = super::interrupts::current_apic_id();
    let mut slots = HOST_SEGMENTS.lock();
    let slot_index = if let Some(slot_index) = slots.iter().position(|slot| slot.apic_id == apic_id) {
        slot_index
    } else {
        slots.push(Box::new(HostSegmentSlot::new()));
        slots.len() - 1
    };
    let slot = slots[slot_index].as_mut();
    if slot.apic_id != apic_id {
        slot.apic_id = apic_id;
    }
    let gdt_base = addr_of!(slot.gdt) as u64;
    let tss_base = addr_of!(slot.tss) as u64;
    if slot.initialized {
        write_msr(IA32_GS_BASE, addr_of!(*slot) as u64);
        write_msr(IA32_KERNEL_GS_BASE, 0);
        return Some(HostSegmentState {
            gdt_base,
            tss_base,
            tss_selector: HOST_TSS_SELECTOR,
        });
    }
    let tss_limit = (size_of::<TaskStateSegment>() - 1) as u64;
    let tss_descriptor = (tss_limit & 0xffff)
        | ((tss_base & 0xffff) << 16)
        | (((tss_base >> 16) & 0xff) << 32)
        | (0x89 << 40)
        | (((tss_limit >> 16) & 0xf) << 48)
        | (((tss_base >> 24) & 0xff) << 56);
    unsafe {
        let gdt = addr_of_mut!(slot.gdt).cast::<u64>();
        write_volatile(gdt.add(3), tss_descriptor);
        write_volatile(gdt.add(4), tss_base >> 32);
        let pointer = DescriptorTablePointer {
            limit: (size_of::<[u64; 7]>() - 1) as u16,
            base: gdt_base,
        };
        core::arch::asm!(
            "lgdt [{pointer}]",
            pointer = in(reg) &pointer,
            options(readonly, nostack, preserves_flags),
        );
        core::arch::asm!(
            "ltr {selector:x}",
            selector = in(reg) HOST_TSS_SELECTOR,
            options(nostack, preserves_flags),
        );
    }
    slot.initialized = true;
    write_msr(IA32_GS_BASE, addr_of!(*slot) as u64);
    write_msr(IA32_KERNEL_GS_BASE, 0);
    Some(HostSegmentState {
        gdt_base,
        tss_base,
        tss_selector: HOST_TSS_SELECTOR,
    })
}

pub(crate) fn set_kernel_stack(stack_pointer: usize) -> bool {
    let apic_id = super::interrupts::current_apic_id();
    let initialized = HOST_SEGMENTS
        .lock()
        .iter()
        .any(|slot| slot.apic_id == apic_id);
    if !initialized && initialize().is_none() {
        return false;
    }
    let mut slots = HOST_SEGMENTS.lock();
    let Some(slot) = slots.iter_mut().find(|slot| slot.apic_id == apic_id) else {
        return false;
    };
    slot.kernel_rsp = stack_pointer as u64;
    let tss_stack = unsafe { addr_of_mut!(slot.tss).cast::<u8>().add(4).cast::<u32>() };
    let stack_pointer = stack_pointer as u64;
    unsafe {
        write_volatile(tss_stack, stack_pointer as u32);
        write_volatile(tss_stack.add(1), (stack_pointer >> 32) as u32);
    }
    true
}

fn write_msr(index: u32, value: u64) {
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") index,
            in("eax") value as u32,
            in("edx") (value >> 32) as u32,
            options(nostack, preserves_flags),
        );
    }
}