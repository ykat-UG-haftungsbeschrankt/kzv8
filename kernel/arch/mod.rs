use core::sync::atomic::{AtomicUsize, Ordering};

#[cfg(target_arch = "aarch64")]
#[path = "aarch64/smp.rs"]
mod smp;
#[cfg(target_arch = "x86_64")]
#[path = "x86_64/smp.rs"]
mod smp;

pub(crate) const MAX_CPUS: usize = 8;
pub(crate) const CPU_STACK_SIZE: usize = 16 * 1024;

#[repr(C, align(16))]
pub(crate) struct CpuStack([u8; CPU_STACK_SIZE]);

#[no_mangle]
pub(crate) static mut KZ_CPU_STACKS: [CpuStack; MAX_CPUS] =
    [const { CpuStack([0; CPU_STACK_SIZE]) }; MAX_CPUS];

static ONLINE_CPUS: AtomicUsize = AtomicUsize::new(0);

#[cfg(target_arch = "aarch64")]
#[path = "aarch64/entry.rs"]
mod entry;
#[cfg(target_arch = "x86_64")]
#[path = "x86_64/entry.rs"]
mod entry;
#[cfg(target_arch = "aarch64")]
#[path = "aarch64/interrupts.rs"]
pub(crate) mod interrupts;
#[cfg(target_arch = "x86_64")]
#[path = "x86_64/interrupts.rs"]
pub(crate) mod interrupts;
#[cfg(target_arch = "aarch64")]
#[path = "aarch64/paging.rs"]
mod paging;
#[cfg(target_arch = "x86_64")]
#[path = "x86_64/paging.rs"]
mod paging;
#[cfg(target_arch = "aarch64")]
#[path = "aarch64/ram_map.rs"]
mod ram_map;
#[cfg(target_arch = "x86_64")]
#[path = "x86_64/ram_map.rs"]
mod ram_map;
#[cfg(target_arch = "aarch64")]
#[path = "aarch64/threads.rs"]
mod threads;
#[cfg(target_arch = "x86_64")]
#[path = "x86_64/threads.rs"]
mod threads;
#[cfg(target_arch = "aarch64")]
#[path = "aarch64/timer.rs"]
pub(crate) mod timer;
#[cfg(target_arch = "x86_64")]
#[path = "x86_64/timer.rs"]
pub(crate) mod timer;

pub(crate) fn initialize_mmu() {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        paging::initialize();
        ram_map::map_memory_regions(crate::platform::memory_regions());
    }
    #[cfg(target_arch = "x86_64")]
    {
        paging::initialize();
        unsafe { ram_map::map_high_memory() };
    }
}

pub(crate) fn initialize_thread_stack(stack: &mut [u8], entry: extern "C" fn() -> !) -> usize {
    threads::initialize_stack(stack, entry)
}

pub(crate) fn switch_thread(old_stack: &mut usize, new_stack: usize) {
    unsafe { threads::switch(old_stack, new_stack) }
}

pub(crate) fn initialize_interrupt_controller() -> bool {
    interrupts::initialize_controller()
}

pub(crate) fn start_secondary_cpus() -> usize {
    ONLINE_CPUS.store(1, Ordering::Release);
    smp::start_secondary_cpus();
    online_cpu_count()
}

pub(crate) fn initialize_secondary_cpu(cpu_id: usize) -> bool {
    smp::initialize_secondary_cpu(cpu_id)
}

pub(crate) fn mark_cpu_online(cpu_id: usize) {
    if cpu_id < MAX_CPUS {
        ONLINE_CPUS.fetch_or(1usize << cpu_id, Ordering::AcqRel);
    }
}

pub(crate) fn cpu_is_online(cpu_id: usize) -> bool {
    cpu_id < MAX_CPUS && ONLINE_CPUS.load(Ordering::Acquire) & (1usize << cpu_id) != 0
}

pub(crate) fn online_cpu_count() -> usize {
    ONLINE_CPUS.load(Ordering::Acquire).count_ones() as usize
}

#[cfg(target_arch = "x86_64")]
pub(crate) fn set_page_permissions(address: usize, length: usize, permissions: i32) -> bool {
    paging::set_page_permissions(address, length, permissions)
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn set_page_permissions(address: usize, length: usize, permissions: i32) -> bool {
    ram_map::set_page_permissions(address, length, permissions)
}

pub(crate) fn halt() -> ! {
    loop {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            core::arch::asm!("wfi", options(nomem, nostack));
        }
    }
}

pub(crate) fn map_page(virtual_address: usize, physical_address: usize, permissions: i32) -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        paging::map_page(virtual_address, physical_address, permissions)
    }
    #[cfg(target_arch = "aarch64")]
    {
        ram_map::map_page(virtual_address, physical_address, permissions)
    }
}

pub(crate) fn unmap_page(virtual_address: usize) -> Option<usize> {
    #[cfg(target_arch = "x86_64")]
    {
        paging::unmap_page(virtual_address)
    }
    #[cfg(target_arch = "aarch64")]
    {
        ram_map::unmap_page(virtual_address)
    }
}

pub(crate) fn page_is_mapped(virtual_address: usize) -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        paging::page_is_mapped(virtual_address)
    }
    #[cfg(target_arch = "aarch64")]
    {
        ram_map::page_is_mapped(virtual_address)
    }
}

pub(crate) fn notify_executor() {
    smp::notify_executor();
}

pub(crate) fn wait_for_interrupt() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("sti", "hlt");
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("msr daifclr, #0xf", "wfe");
    }
}
