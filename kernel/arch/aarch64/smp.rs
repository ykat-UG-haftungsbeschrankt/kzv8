/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::arch::asm;

const PSCI_CPU_ON_64: u64 = 0xc400_0003;
const MPIDR_AFFINITY_MASK: u64 = 0x0000_00ff_00ff_ffff;

unsafe extern "C" {
    fn aarch64_secondary_entry();
}

pub(crate) fn start_secondary_cpus() {
    let Some(conduit) = crate::arch::device_tree::psci_conduit() else {
        return;
    };
    let boot_cpu = current_mpidr() & MPIDR_AFFINITY_MASK;
    let mut slot = 1usize;
    let (cpu_ids, cpu_count) = crate::arch::device_tree::aarch64_cpu_ids();
    for target in cpu_ids.iter().take(cpu_count) {
        if target & MPIDR_AFFINITY_MASK == boot_cpu {
            continue;
        }
        if slot >= super::MAX_CPUS {
            break;
        }
        let result = psci_cpu_on(conduit, *target, slot);
        if result == 0 {
            for _ in 0..20_000_000 {
                if super::cpu_is_online(slot) {
                    break;
                }
                core::hint::spin_loop();
            }
        }
        slot += 1;
    }
}

pub(crate) fn initialize_secondary_cpu(cpu_id: usize) -> bool {
    if cpu_id == 0 || cpu_id >= super::MAX_CPUS {
        return false;
    }
    unsafe {
        super::initialize_mmu();
        super::interrupts::initialize();
    }
    super::interrupts::initialize_secondary_controller()
}

pub(crate) fn notify_executor() {
    unsafe { asm!("sev", options(nostack, preserves_flags)) };
}

fn psci_cpu_on(conduit: crate::arch::device_tree::PsciConduit, target: u64, slot: usize) -> i64 {
    let entry = aarch64_secondary_entry as *const () as u64;
    let result: u64;
    unsafe {
        match conduit {
            crate::arch::device_tree::PsciConduit::Smc => asm!(
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
            crate::arch::device_tree::PsciConduit::Hvc => asm!(
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
