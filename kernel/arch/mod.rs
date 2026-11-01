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
#[path = "aarch64/timer.rs"]
pub(crate) mod timer;
#[cfg(target_arch = "x86_64")]
#[path = "x86_64/timer.rs"]
pub(crate) mod timer;

pub(crate) fn initialize_mmu() {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        paging::initialize();
    }
    #[cfg(target_arch = "x86_64")]
    paging::initialize();
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
