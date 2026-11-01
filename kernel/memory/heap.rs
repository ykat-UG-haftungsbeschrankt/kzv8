use core::alloc::{GlobalAlloc, Layout};
use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicUsize, Ordering};

const HEAP_SIZE: usize = 256 * 1024;

#[repr(C, align(4096))]
struct Heap([u8; HEAP_SIZE]);

static mut HEAP: Heap = Heap([0; HEAP_SIZE]);
static NEXT: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct BumpAllocator;

#[global_allocator]
static GLOBAL_ALLOCATOR: BumpAllocator = BumpAllocator;

impl BumpAllocator {
    unsafe fn allocate(&self, layout: Layout) -> *mut u8 {
        let base = addr_of_mut!(HEAP.0).cast::<u8>() as usize;
        let mut current = NEXT.load(Ordering::Relaxed);
        loop {
            let aligned = (base + current + layout.align() - 1) & !(layout.align() - 1);
            let Some(end) = aligned.checked_add(layout.size()) else {
                return core::ptr::null_mut();
            };
            let next = end - base;
            if next > HEAP_SIZE {
                return core::ptr::null_mut();
            }
            match NEXT.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Relaxed) {
                Ok(_) => return aligned as *mut u8,
                Err(updated) => current = updated,
            }
        }
    }
}

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.allocate(layout)
    }

    unsafe fn dealloc(&self, _pointer: *mut u8, _layout: Layout) {}
}

pub(crate) fn initialize() {
    NEXT.store(0, Ordering::Release);
}

pub(crate) fn remaining() -> usize {
    HEAP_SIZE.saturating_sub(NEXT.load(Ordering::Acquire))
}
