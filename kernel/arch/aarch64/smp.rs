/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::kernel::arch::traits::{CpuArchitecture as _, ThreadArchitecture as _};

const PSCI_CPU_ON_64: u64 = 0xc400_0003;
const MPIDR_AFFINITY_MASK: u64 = 0x0000_00ff_00ff_ffff;
const CPU_STACK_SIZE: usize = 16 * 1024;
const CPU_ONLINE_TIMEOUT_NS: u64 = 1_000_000_000;

#[repr(C, align(16))]
struct CpuStack([u8; CPU_STACK_SIZE]);

#[no_mangle]
static mut KZ_CPU_STACKS: [CpuStack; super::MAX_CPUS] =
    [const { CpuStack([0; CPU_STACK_SIZE]) }; super::MAX_CPUS];

static CPU_ONLINE: [AtomicBool; super::MAX_CPUS] =
    [const { AtomicBool::new(false) }; super::MAX_CPUS];

unsafe extern "C" {
    fn aarch64_secondary_entry();
}

pub(crate) fn start_secondary_cpus() -> usize {
    CPU_ONLINE[0].store(true, Ordering::Release);
    let Some(conduit) = super::device_tree::psci_conduit() else {
        return online_cpu_count();
    };
    let Some(frequency_hz) = super::timer::frequency_hz() else {
        return online_cpu_count();
    };
    let Some(timeout_ticks) = duration_ticks(frequency_hz, CPU_ONLINE_TIMEOUT_NS) else {
        return online_cpu_count();
    };
    let boot_cpu = current_mpidr() & MPIDR_AFFINITY_MASK;
    let mut slot = 1usize;
    let (cpu_ids, cpu_count) = super::device_tree::aarch64_cpu_ids();
    let mut started = [false; super::MAX_CPUS];
    for target in cpu_ids.iter().take(cpu_count) {
        if target & MPIDR_AFFINITY_MASK == boot_cpu {
            continue;
        }
        if slot >= super::MAX_CPUS {
            break;
        }
        let result = psci_cpu_on(conduit, *target, slot);
        if result == 0 {
            started[slot] = true;
        }
        slot += 1;
    }
    if !started.iter().any(|started| *started) {
        return online_cpu_count();
    }

    let start = super::timer::ticks();
    while !all_started_online(&started)
        && super::timer::ticks().wrapping_sub(start) < timeout_ticks
    {
        core::hint::spin_loop();
    }
    online_cpu_count()
}

pub(crate) fn initialize_secondary_cpu(cpu_id: usize) -> bool {
    if cpu_id == 0 || cpu_id >= super::MAX_CPUS {
        return false;
    }
    unsafe {
        super::paging::initialize_secondary();
        super::interrupts::initialize();
    }
    if !crate::kernel::arch::ThreadArchitecture::initialize_thread_local_storage(cpu_id) {
        return false;
    }
    super::interrupts::initialize_secondary_controller()
}

pub(crate) fn notify_executor() {
    unsafe { asm!("sev", options(nostack, preserves_flags)) };
}

fn set_cpu_online(cpu_id: usize) -> bool {
    let Some(online) = CPU_ONLINE.get(cpu_id) else {
        return false;
    };
    online.store(true, Ordering::Release);
    notify_executor();
    true
}

fn all_started_online(started: &[bool; super::MAX_CPUS]) -> bool {
    started.iter().enumerate().all(|(slot, started)| {
        !*started || CPU_ONLINE[slot].load(Ordering::Acquire)
    })
}

fn online_cpu_count() -> usize {
    CPU_ONLINE
        .iter()
        .filter(|online| online.load(Ordering::Acquire))
        .count()
}

#[no_mangle]
pub extern "C" fn kz_secondary_main(cpu_id: usize) -> ! {
    if !initialize_secondary_cpu(cpu_id) || !set_cpu_online(cpu_id) {
        crate::kernel::arch::CpuArchitecture::halt();
    }
    loop {
        crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
    }
}

fn duration_ticks(frequency_hz: u64, duration_ns: u64) -> Option<u64> {
    let ticks = (u128::from(frequency_hz) * u128::from(duration_ns)).div_ceil(1_000_000_000);
    u64::try_from(ticks.max(1)).ok()
}

fn psci_cpu_on(conduit: super::device_tree::PsciConduit, target: u64, slot: usize) -> i64 {
    let entry = aarch64_secondary_entry as *const () as u64;
    let result: u64;
    unsafe {
        match conduit {
            super::device_tree::PsciConduit::Smc => asm!(
                "smc #0",
                inout("x0") PSCI_CPU_ON_64 => result,
                inlateout("x1") target => _,
                inlateout("x2") entry => _,
                inlateout("x3") slot as u64 => _,
                lateout("x4") _,
                lateout("x5") _,
                lateout("x6") _,
                lateout("x7") _,
                lateout("x8") _,
                lateout("x9") _,
                lateout("x10") _,
                lateout("x11") _,
                lateout("x12") _,
                lateout("x13") _,
                lateout("x14") _,
                lateout("x15") _,
                lateout("x16") _,
                lateout("x17") _,
                options(nostack),
            ),
            super::device_tree::PsciConduit::Hvc => asm!(
                "hvc #0",
                inout("x0") PSCI_CPU_ON_64 => result,
                inlateout("x1") target => _,
                inlateout("x2") entry => _,
                inlateout("x3") slot as u64 => _,
                lateout("x4") _,
                lateout("x5") _,
                lateout("x6") _,
                lateout("x7") _,
                lateout("x8") _,
                lateout("x9") _,
                lateout("x10") _,
                lateout("x11") _,
                lateout("x12") _,
                lateout("x13") _,
                lateout("x14") _,
                lateout("x15") _,
                lateout("x16") _,
                lateout("x17") _,
                options(nostack),
            ),
        }
    }
    result as i64
}

fn current_mpidr() -> u64 {
    let value: u64;
    unsafe {
        asm!(
            "mrs {value}, mpidr_el1",
            value = out(reg) value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}
