/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::arch::x86_64::{__cpuid, __cpuid_count};
use core::ptr::{addr_of, copy_nonoverlapping, write_volatile};
use core::sync::atomic::{fence, AtomicBool, Ordering};

use crate::kernel::arch::traits::{
    CpuArchitecture as _, DebugArchitecture as _, ThreadArchitecture as _,
};
use crate::kernel::scheduler::sync::IrqSpinLock;

const AP_TRAMPOLINE_ADDRESS: usize = 0x8000;
const PAGE_SIZE: usize = 4096;
const CPU_STACK_SIZE: usize = 16 * 1024;
const INIT_IPI: u32 = 0x0000_c500;
const STARTUP_IPI_BASE: u32 = 0x0000_0600;
const EXECUTOR_IPI_VECTOR: u8 = 0x41;

#[repr(C, align(16))]
struct CpuStack([u8; CPU_STACK_SIZE]);

struct CpuSlot {
    apic_id: u32,
    stack: Option<Box<CpuStack>>,
    online: AtomicBool,
}

static CPU_SLOTS: IrqSpinLock<Vec<CpuSlot>> = IrqSpinLock::new(Vec::new());

unsafe extern "C" {
    static ap_trampoline_start: u8;
    static ap_trampoline_end: u8;
    static ap_trampoline_stack: u64;
    static ap_trampoline_slot: u32;
}

pub(crate) fn start_secondary_cpus() -> usize {
    trace(b'b');
    let bsp_apic_id = crate::kernel::arch::interrupts::current_apic_id();
    let mut apic_ids = super::acpi::cpu_apic_ids().unwrap_or_else(detected_cpu_ids);
    if apic_ids.is_empty() {
        apic_ids = detected_cpu_ids();
    }
    apic_ids.retain(|apic_id| *apic_id != bsp_apic_id);
    apic_ids.sort_unstable();
    apic_ids.dedup();
    apic_ids.insert(0, bsp_apic_id);

    let mut cpu_slots = Vec::with_capacity(apic_ids.len());
    for (slot, apic_id) in apic_ids.into_iter().enumerate() {
        cpu_slots.push(CpuSlot {
            apic_id,
            stack: (slot != 0).then(|| Box::new(CpuStack([0; CPU_STACK_SIZE]))),
            online: AtomicBool::new(slot == 0),
        });
    }
    *CPU_SLOTS.lock() = cpu_slots;
    trace(b'c');

    let cpu_count = CPU_SLOTS.lock().len();
    if cpu_count < 2 {
        return cpu_count;
    }

    let start = addr_of!(ap_trampoline_start);
    let length = addr_of!(ap_trampoline_end) as usize - start as usize;
    if length == 0 || length > PAGE_SIZE {
        return online_cpu_count();
    }
    // SAFETY: the low-memory trampoline page is identity mapped for real-mode AP startup.
    unsafe { copy_nonoverlapping(start, AP_TRAMPOLINE_ADDRESS as *mut u8, length) };

    let stack_offset = addr_of!(ap_trampoline_stack) as usize - start as usize;
    let slot_offset = addr_of!(ap_trampoline_slot) as usize - start as usize;
    for slot in 1..cpu_count {
        let Some(trampoline_slot) = u32::try_from(slot).ok() else {
            continue;
        };
        let (apic_id, stack) = {
            let cpus = CPU_SLOTS.lock();
            let Some(cpu) = cpus.get(slot) else {
                continue;
            };
            let Some(stack) = cpu.stack.as_ref() else {
                continue;
            };
            (cpu.apic_id, stack.0.as_ptr() as usize + CPU_STACK_SIZE)
        };
        unsafe {
            write_volatile((AP_TRAMPOLINE_ADDRESS + stack_offset) as *mut u64, stack as u64);
            write_volatile(
                (AP_TRAMPOLINE_ADDRESS + slot_offset) as *mut u32,
                trampoline_slot,
            );
        }
        fence(Ordering::SeqCst);

        trace(b'd');
        if !crate::kernel::arch::interrupts::send_ipi(apic_id, INIT_IPI) {
            continue;
        }
        wait_for_ipi_delivery();
        delay(10_000_000);
        trace(b'e');
        if !crate::kernel::arch::interrupts::send_ipi(
            apic_id,
            STARTUP_IPI_BASE | (AP_TRAMPOLINE_ADDRESS as u32 >> 12),
        ) {
            continue;
        }
        wait_for_ipi_delivery();
        delay(200_000);
        trace(b'f');
        let _ = crate::kernel::arch::interrupts::send_ipi(
            apic_id,
            STARTUP_IPI_BASE | (AP_TRAMPOLINE_ADDRESS as u32 >> 12),
        );
        wait_for_ipi_delivery();
        trace(b'g');

        for _ in 0..20_000_000 {
            if cpu_is_online(slot) {
                break;
            }
            core::hint::spin_loop();
        }
        trace(b'h');
    }
    online_cpu_count()
}

pub(crate) fn initialize_secondary_cpu(cpu_id: usize) -> bool {
    if cpu_id == 0 || CPU_SLOTS.lock().get(cpu_id).is_none() {
        return false;
    }
    unsafe { crate::kernel::arch::interrupts::initialize_secondary() };
    crate::kernel::arch::interrupts::initialize_local_controller()
        && crate::kernel::arch::ThreadArchitecture::initialize_thread_local_storage(cpu_id)
}

fn set_cpu_online(cpu_id: usize) -> bool {
    let cpus = CPU_SLOTS.lock();
    let Some(cpu) = cpus.get(cpu_id) else {
        return false;
    };
    cpu.online.store(true, Ordering::Release);
    true
}

fn cpu_is_online(cpu_id: usize) -> bool {
    CPU_SLOTS
        .lock()
        .get(cpu_id)
        .is_some_and(|cpu| cpu.online.load(Ordering::Acquire))
}

fn online_cpu_count() -> usize {
    CPU_SLOTS
        .lock()
        .iter()
        .filter(|cpu| cpu.online.load(Ordering::Acquire))
        .count()
}

#[no_mangle]
pub extern "C" fn kz_secondary_main(cpu_id: usize) -> ! {
    trace(b'A');
    if !initialize_secondary_cpu(cpu_id) || !set_cpu_online(cpu_id) {
        crate::kernel::arch::CpuArchitecture::halt();
    }
    trace(b'C');
    loop {
        crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
    }
}

fn trace(code: u8) {
    crate::kernel::arch::DebugArchitecture::decommit_debug(code);
}

pub(crate) fn notify_executor() {
    for cpu in CPU_SLOTS.lock().iter() {
        if cpu.online.load(Ordering::Acquire) {
            let _ = crate::kernel::arch::interrupts::send_ipi(
                cpu.apic_id,
                u32::from(EXECUTOR_IPI_VECTOR),
            );
        }
    }
}

fn detected_cpu_ids() -> Vec<u32> {
    let mut apic_ids = Vec::with_capacity(detected_cpu_count());
    for cpu_id in 0..detected_cpu_count() {
        if let Ok(apic_id) = u32::try_from(cpu_id) {
            apic_ids.push(apic_id);
        }
    }
    apic_ids
}

fn detected_cpu_count() -> usize {
    let maximum_leaf = unsafe { __cpuid(0).eax };
    let mut count = 1usize;
    if maximum_leaf >= 0x0b {
        for level in 0..8 {
            let topology = unsafe { __cpuid_count(0x0b, level) };
            if (topology.ecx >> 8) & 0xff == 0 {
                break;
            }
            count = count.max((topology.ebx & 0xffff) as usize);
        }
    }
    if count == 1 {
        count = ((unsafe { __cpuid(1).ebx } >> 16) & 0xff) as usize;
    }
    count.max(1)
}

fn wait_for_ipi_delivery() {
    for _ in 0..1_000_000 {
        if !crate::kernel::arch::interrupts::ipi_pending() {
            return;
        }
        core::hint::spin_loop();
    }
}

fn delay(iterations: usize) {
    for _ in 0..iterations {
        core::hint::spin_loop();
    }
}
