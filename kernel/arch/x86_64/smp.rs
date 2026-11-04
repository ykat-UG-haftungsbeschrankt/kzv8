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
const INIT_TO_SIPI_NS: u64 = 10_000_000;
const SIPI_INTERVAL_NS: u64 = 200_000;
const IPI_DELIVERY_TIMEOUT_NS: u64 = 10_000_000;
const CPU_ONLINE_TIMEOUT_NS: u64 = 1_000_000_000;

#[repr(C, align(16))]
struct CpuStack([u8; CPU_STACK_SIZE]);

struct CpuSlot {
    apic_id: u32,
    stack: Option<Box<CpuStack>>,
    online: AtomicBool,
}

#[repr(C)]
struct ApBootRecord {
    apic_id: u32,
    slot: u32,
    stack: u64,
}

static CPU_SLOTS: IrqSpinLock<Vec<CpuSlot>> = IrqSpinLock::new(Vec::new());
static AP_BOOT_RECORDS: IrqSpinLock<Vec<ApBootRecord>> = IrqSpinLock::new(Vec::new());

unsafe extern "C" {
    static ap_trampoline_start: u8;
    static ap_trampoline_end: u8;
    static ap_trampoline_records_ptr: u64;
    static ap_trampoline_records_len: u32;
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
    let Some(tsc_frequency) = super::timer::frequency_hz() else {
        return online_cpu_count();
    };
    let Some(init_delay_ticks) = duration_ticks(tsc_frequency, INIT_TO_SIPI_NS) else {
        return online_cpu_count();
    };
    let Some(sipi_interval_ticks) = duration_ticks(tsc_frequency, SIPI_INTERVAL_NS) else {
        return online_cpu_count();
    };
    let Some(ipi_timeout_ticks) = duration_ticks(tsc_frequency, IPI_DELIVERY_TIMEOUT_NS) else {
        return online_cpu_count();
    };
    let Some(online_timeout_ticks) = duration_ticks(tsc_frequency, CPU_ONLINE_TIMEOUT_NS) else {
        return online_cpu_count();
    };

    let start = addr_of!(ap_trampoline_start);
    let length = addr_of!(ap_trampoline_end) as usize - start as usize;
    if length == 0 || length > PAGE_SIZE {
        return online_cpu_count();
    }
    // SAFETY: the low-memory trampoline page is identity mapped for real-mode AP startup.
    unsafe { copy_nonoverlapping(start, AP_TRAMPOLINE_ADDRESS as *mut u8, length) };

    let boot_records = {
        let cpus = CPU_SLOTS.lock();
        cpus.iter()
            .enumerate()
            .skip(1)
            .filter_map(|(slot, cpu)| {
                let slot = u32::try_from(slot).ok()?;
                let stack = cpu.stack.as_ref()?;
                Some(ApBootRecord {
                    apic_id: cpu.apic_id,
                    slot,
                    stack: (stack.0.as_ptr() as usize + CPU_STACK_SIZE) as u64,
                })
            })
            .collect::<Vec<_>>()
    };
    let Ok(records_len) = u32::try_from(boot_records.len()) else {
        return online_cpu_count();
    };
    let records_ptr = {
        let mut records = AP_BOOT_RECORDS.lock();
        *records = boot_records;
        records.as_ptr() as u64
    };
    let records_ptr_offset = addr_of!(ap_trampoline_records_ptr) as usize - start as usize;
    let records_len_offset = addr_of!(ap_trampoline_records_len) as usize - start as usize;
    unsafe {
        write_volatile(
            (AP_TRAMPOLINE_ADDRESS + records_ptr_offset) as *mut u64,
            records_ptr,
        );
        write_volatile(
            (AP_TRAMPOLINE_ADDRESS + records_len_offset) as *mut u32,
            records_len,
        );
    }
    fence(Ordering::SeqCst);

    let mut initialized = Vec::with_capacity(cpu_count);
    initialized.resize(cpu_count, false);
    let mut init_delivery_ok = true;
    for slot in 1..cpu_count {
        let Some(apic_id) = CPU_SLOTS.lock().get(slot).map(|cpu| cpu.apic_id) else {
            continue;
        };
        trace(b'd');
        if !crate::kernel::arch::interrupts::send_ipi(apic_id, INIT_IPI) {
            continue;
        }
        if !wait_for_ipi_delivery(ipi_timeout_ticks) {
            init_delivery_ok = false;
            break;
        }
        initialized[slot] = true;
    }
    if !init_delivery_ok || !initialized.iter().any(|initialized| *initialized) {
        return online_cpu_count();
    }
    wait_for_duration(init_delay_ticks);
    trace(b'e');

    let mut started = Vec::with_capacity(cpu_count);
    started.resize(cpu_count, false);
    let mut delivery_ok = true;
    for slot in 1..cpu_count {
        if !initialized[slot] {
            continue;
        }
        let Some(apic_id) = CPU_SLOTS.lock().get(slot).map(|cpu| cpu.apic_id) else {
            continue;
        };
        if !crate::kernel::arch::interrupts::send_ipi(
            apic_id,
            STARTUP_IPI_BASE | (AP_TRAMPOLINE_ADDRESS as u32 >> 12),
        ) {
            continue;
        }
        started[slot] = true;
        if !wait_for_ipi_delivery(ipi_timeout_ticks) {
            delivery_ok = false;
            break;
        }
    }
    if delivery_ok && started.iter().any(|started| *started) {
        wait_for_duration(sipi_interval_ticks);
        trace(b'f');
        for slot in 1..cpu_count {
            if !started[slot] {
                continue;
            }
            let Some(apic_id) = CPU_SLOTS.lock().get(slot).map(|cpu| cpu.apic_id) else {
                continue;
            };
            if !crate::kernel::arch::interrupts::send_ipi(
                apic_id,
                STARTUP_IPI_BASE | (AP_TRAMPOLINE_ADDRESS as u32 >> 12),
            ) {
                continue;
            }
            if !wait_for_ipi_delivery(ipi_timeout_ticks) {
                break;
            }
        }
        trace(b'g');
    }

    let start = super::timer::ticks();
    while super::timer::ticks().wrapping_sub(start) < online_timeout_ticks
        && !all_started_cpus_online(&started)
    {
        core::hint::spin_loop();
    }
    trace(b'h');
    online_cpu_count()
}

pub(crate) fn initialize_secondary_cpu(cpu_id: usize) -> bool {
    trace(b'B');
    if cpu_id == 0 || CPU_SLOTS.lock().get(cpu_id).is_none() {
        return false;
    }
    unsafe { crate::kernel::arch::interrupts::initialize_secondary() };
    trace(b'D');
    if !crate::kernel::arch::interrupts::initialize_local_controller() {
        return false;
    }
    trace(b'E');
    let tls_initialized =
        crate::kernel::arch::ThreadArchitecture::initialize_thread_local_storage(cpu_id);
    trace(b'F');
    tls_initialized
}

fn set_cpu_online(cpu_id: usize) -> bool {
    let cpus = CPU_SLOTS.lock();
    let Some(cpu) = cpus.get(cpu_id) else {
        return false;
    };
    cpu.online.store(true, Ordering::Release);
    true
}

fn all_started_cpus_online(started: &[bool]) -> bool {
    let cpus = CPU_SLOTS.lock();
    started.iter().enumerate().all(|(slot, started)| {
        !*started
            || cpus
                .get(slot)
                .is_some_and(|cpu| cpu.online.load(Ordering::Acquire))
    })
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

fn duration_ticks(frequency_hz: u64, duration_ns: u64) -> Option<u64> {
    let ticks = (u128::from(frequency_hz) * u128::from(duration_ns)).div_ceil(1_000_000_000);
    u64::try_from(ticks.max(1)).ok()
}

fn wait_for_duration(duration_ticks: u64) {
    let start = super::timer::ticks();
    while super::timer::ticks().wrapping_sub(start) < duration_ticks {
        core::hint::spin_loop();
    }
}

fn wait_for_ipi_delivery(timeout_ticks: u64) -> bool {
    let start = super::timer::ticks();
    while crate::kernel::arch::interrupts::ipi_pending() {
        if super::timer::ticks().wrapping_sub(start) >= timeout_ticks {
            return false;
        }
        core::hint::spin_loop();
    }
    true
}
