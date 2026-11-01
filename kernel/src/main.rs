#![no_std]
#![no_main]

use core::ptr::{addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

const QUEUE_SIZE: usize = 8;

#[repr(C)]
#[derive(Clone, Copy)]
struct Descriptor {
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
}

#[repr(C, align(4096))]
struct Descriptors([Descriptor; QUEUE_SIZE]);

#[repr(C, align(4096))]
struct Available {
    flags: u16,
    index: u16,
    ring: [u16; QUEUE_SIZE],
    used_event: u16,
}

#[repr(C, align(4096))]
struct Used {
    flags: u16,
    index: u16,
    ring: [(u32, u32); QUEUE_SIZE],
    available_event: u16,
}

static mut DESCRIPTORS: Descriptors = Descriptors([Descriptor {
    address: 0,
    length: 0,
    flags: 0,
    next: 0,
}; QUEUE_SIZE]);
static mut AVAILABLE: Available = Available {
    flags: 0,
    index: 0,
    ring: [0; QUEUE_SIZE],
    used_event: 0,
};
static mut USED: Used = Used {
    flags: 0,
    index: 0,
    ring: [(0, 0); QUEUE_SIZE],
    available_event: 0,
};
static mut OUTPUT: [u8; 512] = [0; 512];

unsafe fn register_read(base: usize, offset: usize) -> u32 {
    read_volatile((base + offset) as *const u32)
}

unsafe fn register_write(base: usize, offset: usize, value: u32) {
    write_volatile((base + offset) as *mut u32, value);
}

fn dma_barrier() {
    fence(Ordering::SeqCst);
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("dsb sy", options(nostack, preserves_flags));
    }
}

unsafe fn queue_address(base: usize, offset: usize, address: u64) {
    register_write(base, offset, address as u32);
    register_write(base, offset + 4, (address >> 32) as u32);
}

unsafe fn find_console() -> Option<usize> {
    #[cfg(target_arch = "x86_64")]
    let (start, count) = (0xfeb0_0000usize, 24usize);
    #[cfg(target_arch = "aarch64")]
    let (start, count) = (0x0a00_0000usize, 32usize);

    for slot in 0..count {
        let base = start + slot * 0x200;
        if register_read(base, 0) == 0x7472_6976
            && register_read(base, 4) == 2
            && register_read(base, 8) == 3
        {
            return Some(base);
        }
    }
    None
}

unsafe fn initialize_console(base: usize) -> bool {
    register_write(base, 0x70, 0);
    register_write(base, 0x70, 1);
    register_write(base, 0x70, 3);
    register_write(base, 0x14, 1);
    if register_read(base, 0x10) & 1 == 0 {
        return false;
    }
    register_write(base, 0x24, 0);
    register_write(base, 0x20, 0);
    register_write(base, 0x24, 1);
    register_write(base, 0x20, 1);
    register_write(base, 0x70, 11);
    if register_read(base, 0x70) & 8 == 0 {
        return false;
    }
    register_write(base, 0x30, 1);
    if register_read(base, 0x34) < QUEUE_SIZE as u32 {
        return false;
    }
    register_write(base, 0x38, QUEUE_SIZE as u32);
    queue_address(base, 0x80, addr_of_mut!(DESCRIPTORS) as u64);
    queue_address(base, 0x90, addr_of_mut!(AVAILABLE) as u64);
    queue_address(base, 0xa0, addr_of_mut!(USED) as u64);
    dma_barrier();
    register_write(base, 0x44, 1);
    register_write(base, 0x70, 15);
    true
}

unsafe fn console_write(base: usize, message: &[u8]) -> bool {
    if message.len() > 512 {
        return false;
    }
    for (index, byte) in message.iter().enumerate() {
        write_volatile(addr_of_mut!(OUTPUT).cast::<u8>().add(index), *byte);
    }
    write_volatile(addr_of_mut!(DESCRIPTORS.0[0]), Descriptor {
        address: addr_of_mut!(OUTPUT) as u64,
        length: message.len() as u32,
        flags: 0,
        next: 0,
    });
    let next = read_volatile(addr_of_mut!(AVAILABLE.index)).wrapping_add(1);
    write_volatile(addr_of_mut!(AVAILABLE.ring[(next.wrapping_sub(1) as usize) % QUEUE_SIZE]), 0);
    dma_barrier();
    write_volatile(addr_of_mut!(AVAILABLE.index), next);
    dma_barrier();
    register_write(base, 0x50, 1);
    for _attempt in 0..10_000_000 {
        if read_volatile(addr_of_mut!(USED.index)) == next {
            dma_barrier();
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    unsafe {
        if let Some(base) = find_console() {
            if initialize_console(base) {
                #[cfg(target_arch = "x86_64")]
                let message = b"kz: x86_64 Rust kernel booted\n";
                #[cfg(target_arch = "aarch64")]
                let message = b"kz: aarch64 Rust kernel booted\n";
                if console_write(base, message)
                    && console_write(base, b"kz: virtio-console transmit OK\n")
                {
                    let _completed = console_write(base, b"kz: V8 not integrated\n");
                }
            }
        }
    }
    halt()
}

fn halt() -> ! {
    loop {
        #[cfg(target_arch = "x86_64")]
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
        #[cfg(target_arch = "aarch64")]
        unsafe { core::arch::asm!("wfi", options(nomem, nostack)); }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    halt()
}