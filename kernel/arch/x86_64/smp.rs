use core::arch::x86_64::{__cpuid, __cpuid_count};
use core::ptr::{addr_of, addr_of_mut, copy_nonoverlapping, write_volatile};
use core::sync::atomic::{fence, Ordering};

const AP_TRAMPOLINE_ADDRESS: usize = 0x8000;
const PAGE_SIZE: usize = 4096;
const INIT_IPI: u32 = 0x0000_c500;
const STARTUP_IPI_BASE: u32 = 0x0000_0600;
const EXECUTOR_IPI_VECTOR: u8 = 0x41;

unsafe extern "C" {
    static ap_trampoline_start: u8;
    static ap_trampoline_end: u8;
    static ap_trampoline_stack: u64;
    static ap_trampoline_slot: u32;
}

pub(crate) fn start_secondary_cpus() {
    let cpu_count = detected_cpu_count().min(super::MAX_CPUS);
    let _diagnostic = crate::console::write(if cpu_count == 1 {
        b"kz: smp detected 1 cpu\n"
    } else {
        b"kz: smp detected multiple cpus\n"
    });
    if cpu_count < 2 {
        return;
    }

    let start = addr_of!(ap_trampoline_start);
    let length = addr_of!(ap_trampoline_end) as usize - start as usize;
    if length == 0 || length > PAGE_SIZE {
        return;
    }
    unsafe { copy_nonoverlapping(start, AP_TRAMPOLINE_ADDRESS as *mut u8, length) };

    let stack_offset = addr_of!(ap_trampoline_stack) as usize - start as usize;
    let slot_offset = addr_of!(ap_trampoline_slot) as usize - start as usize;
    for slot in 1..cpu_count {
        let stack = unsafe {
            addr_of_mut!(super::KZ_CPU_STACKS)
                .cast::<super::CpuStack>()
                .add(slot)
                .cast::<u8>()
                .add(super::CPU_STACK_SIZE) as usize
        };
        unsafe {
            write_volatile((AP_TRAMPOLINE_ADDRESS + stack_offset) as *mut u64, stack as u64);
            write_volatile((AP_TRAMPOLINE_ADDRESS + slot_offset) as *mut u32, slot as u32);
        }
        fence(Ordering::SeqCst);

        let apic_id = slot as u8;
        crate::arch::interrupts::send_ipi(apic_id, INIT_IPI);
        wait_for_ipi_delivery();
        delay(10_000_000);
        crate::arch::interrupts::send_ipi(
            apic_id,
            STARTUP_IPI_BASE | (AP_TRAMPOLINE_ADDRESS as u32 >> 12),
        );
        wait_for_ipi_delivery();
        delay(200_000);
        crate::arch::interrupts::send_ipi(
            apic_id,
            STARTUP_IPI_BASE | (AP_TRAMPOLINE_ADDRESS as u32 >> 12),
        );
        wait_for_ipi_delivery();

        for _ in 0..20_000_000 {
            if super::cpu_is_online(slot) {
                break;
            }
            core::hint::spin_loop();
        }
    }
}

pub(crate) fn initialize_secondary_cpu(cpu_id: usize) -> bool {
    if cpu_id == 0 || cpu_id >= super::MAX_CPUS {
        return false;
    }
    unsafe { crate::arch::interrupts::initialize_secondary() };
    crate::arch::interrupts::initialize_local_controller()
}

pub(crate) fn notify_executor() {
    for cpu_id in 0..super::MAX_CPUS {
        if super::cpu_is_online(cpu_id) {
            crate::arch::interrupts::send_ipi(cpu_id as u8, u32::from(EXECUTOR_IPI_VECTOR));
        }
    }
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
        if !crate::arch::interrupts::ipi_pending() {
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
