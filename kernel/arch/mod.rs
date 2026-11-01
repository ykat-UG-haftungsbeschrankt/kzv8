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

pub(crate) fn notify_executor() {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("sev");
    }
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
    }
}
