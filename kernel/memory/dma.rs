use core::ptr::addr_of_mut;
use core::sync::atomic::{fence, AtomicUsize, Ordering};

const DMA_ARENA_SIZE: usize = 512 * 1024;

#[repr(C, align(4096))]
struct DmaArena([u8; DMA_ARENA_SIZE]);

static mut ARENA: DmaArena = DmaArena([0; DMA_ARENA_SIZE]);
static NEXT: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct DmaBuffer {
    pointer: *mut u8,
    length: usize,
}

impl DmaBuffer {
    pub(crate) const fn len(&self) -> usize {
        self.length
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub(crate) fn bus_address(&self) -> u64 {
        self.pointer as u64
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.pointer, self.length) }
    }
}

pub(crate) fn initialize() {
    NEXT.store(0, Ordering::Release);
}

pub(crate) fn allocate(length: usize, alignment: usize) -> Option<DmaBuffer> {
    if length == 0 || !alignment.is_power_of_two() {
        return None;
    }
    let base = unsafe { addr_of_mut!(ARENA.0).cast::<u8>() as usize };
    let mut current = NEXT.load(Ordering::Relaxed);
    loop {
        let aligned = (base + current + alignment - 1) & !(alignment - 1);
        let end = aligned.checked_add(length)?;
        let next = end.checked_sub(base)?;
        if next > DMA_ARENA_SIZE {
            return None;
        }
        match NEXT.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => {
                return Some(DmaBuffer {
                    pointer: aligned as *mut u8,
                    length,
                });
            }
            Err(updated) => current = updated,
        }
    }
}

pub(crate) fn sync_for_device() {
    fence(Ordering::SeqCst);
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("dsb sy", options(nostack, preserves_flags));
    }
}

pub(crate) fn sync_for_cpu() {
    fence(Ordering::SeqCst);
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("dsb sy", options(nostack, preserves_flags));
    }
}
