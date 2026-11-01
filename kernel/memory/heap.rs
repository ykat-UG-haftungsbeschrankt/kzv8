use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::mem::align_of;
use core::ptr::{addr_of_mut, null_mut};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::sync::{IrqSpinGuard, IrqSpinLock};

const PAGE_SIZE: usize = crate::memory::PAGE_SIZE;
const CLASS_SIZES: [usize; 8] = [16, 32, 64, 128, 256, 512, 1024, 2048];
const MAX_SLABS: usize = 8192;
const MAX_LARGE_ALLOCATIONS: usize = 512;
const NO_INDEX: usize = usize::MAX;
const EINVAL: i32 = 22;
const ENOMEM: i32 = 12;

#[derive(Clone, Copy)]
struct Slab {
    page: usize,
    class_index: usize,
    used: usize,
    next: usize,
    bitmap: [u64; 4],
    active: bool,
}

impl Slab {
    const EMPTY: Self = Self {
        page: 0,
        class_index: 0,
        used: 0,
        next: NO_INDEX,
        bitmap: [0; 4],
        active: false,
    };
}

#[derive(Clone, Copy)]
struct LargeAllocation {
    address: usize,
    length: usize,
    active: bool,
}

impl LargeAllocation {
    const EMPTY: Self = Self {
        address: 0,
        length: 0,
        active: false,
    };
}

static HEAP_LOCK: IrqSpinLock<()> = IrqSpinLock::new(());
static INITIALIZED: AtomicBool = AtomicBool::new(false);
static mut SLABS: [Slab; MAX_SLABS] = [Slab::EMPTY; MAX_SLABS];
static mut CLASS_HEADS: [usize; CLASS_SIZES.len()] = [NO_INDEX; CLASS_SIZES.len()];
static mut LARGE_ALLOCATIONS: [LargeAllocation; MAX_LARGE_ALLOCATIONS] =
    [LargeAllocation::EMPTY; MAX_LARGE_ALLOCATIONS];

pub(crate) struct SlabAllocator;

#[global_allocator]
static GLOBAL_ALLOCATOR: SlabAllocator = SlabAllocator;

fn lock_heap() -> IrqSpinGuard<'static, ()> {
    HEAP_LOCK.lock()
}

fn rounded_length(length: usize) -> Option<usize> {
    length
        .checked_add(PAGE_SIZE - 1)
        .map(|value| value & !(PAGE_SIZE - 1))
}

impl SlabAllocator {
    unsafe fn allocate(&self, layout: Layout) -> *mut u8 {
        let _guard = lock_heap();
        if !INITIALIZED.load(Ordering::Acquire) {
            return null_mut();
        }
        let size = layout.size().max(1);
        let required = size.max(layout.align());
        if let Some(class_index) = CLASS_SIZES.iter().position(|class| required <= *class) {
            return unsafe { self.allocate_small(class_index) };
        }
        unsafe { self.allocate_large(size, layout.align()) }
    }

    unsafe fn allocate_small(&self, class_index: usize) -> *mut u8 {
        let class_size = CLASS_SIZES[class_index];
        let slot_count = PAGE_SIZE / class_size;
        let heads = addr_of_mut!(CLASS_HEADS).cast::<usize>();
        let slabs = addr_of_mut!(SLABS).cast::<Slab>();
        let mut slab_index = unsafe { *heads.add(class_index) };

        while slab_index != NO_INDEX {
            let slab = unsafe { slabs.add(slab_index) };
            if unsafe { (*slab).used } < slot_count {
                for slot in 0..slot_count {
                    let word = slot / u64::BITS as usize;
                    let mask = 1u64 << (slot % u64::BITS as usize);
                    let bitmap = unsafe { (*slab).bitmap[word] };
                    if bitmap & mask == 0 {
                        unsafe {
                            (*slab).bitmap[word] = bitmap | mask;
                            (*slab).used += 1;
                        }
                        let address = unsafe { (*slab).page } + slot * class_size;
                        return address as *mut u8;
                    }
                }
            }
            slab_index = unsafe { (*slab).next };
        }

        let mut metadata_slot = None;
        for index in 0..MAX_SLABS {
            if !unsafe { (*slabs.add(index)).active } {
                metadata_slot = Some(index);
                break;
            }
        }
        let Some(metadata_slot) = metadata_slot else {
            return null_mut();
        };
        let Some(page) = crate::memory::reserve(PAGE_SIZE, PAGE_SIZE, 2) else {
            return null_mut();
        };
        let head = unsafe { *heads.add(class_index) };
        unsafe {
            slabs.add(metadata_slot).write(Slab {
                page,
                class_index,
                used: 1,
                next: head,
                bitmap: [1, 0, 0, 0],
                active: true,
            });
            *heads.add(class_index) = metadata_slot;
        }
        page as *mut u8
    }

    unsafe fn allocate_large(&self, size: usize, alignment: usize) -> *mut u8 {
        let allocations = addr_of_mut!(LARGE_ALLOCATIONS).cast::<LargeAllocation>();
        let mut metadata_slot = None;
        for index in 0..MAX_LARGE_ALLOCATIONS {
            if !unsafe { (*allocations.add(index)).active } {
                metadata_slot = Some(index);
                break;
            }
        }
        let Some(metadata_slot) = metadata_slot else {
            return null_mut();
        };
        let Some(length) = rounded_length(size) else {
            return null_mut();
        };
        let alignment = alignment.max(PAGE_SIZE);
        let Some(address) = crate::memory::reserve(length, alignment, 2) else {
            return null_mut();
        };
        unsafe {
            allocations.add(metadata_slot).write(LargeAllocation {
                address,
                length,
                active: true,
            });
        }
        address as *mut u8
    }

    unsafe fn deallocate(&self, pointer: *mut u8) {
        if pointer.is_null() {
            return;
        }
        let _guard = lock_heap();
        if !INITIALIZED.load(Ordering::Acquire) {
            return;
        }
        let address = pointer as usize;
        let allocations = addr_of_mut!(LARGE_ALLOCATIONS).cast::<LargeAllocation>();
        for index in 0..MAX_LARGE_ALLOCATIONS {
            let allocation = unsafe { allocations.add(index) };
            if unsafe { (*allocation).active && (*allocation).address == address } {
                let current = unsafe { *allocation };
                if crate::memory::release(current.address, current.length) {
                    unsafe { allocation.write(LargeAllocation::EMPTY) };
                }
                return;
            }
        }

        let page = address & !(PAGE_SIZE - 1);
        let slabs = addr_of_mut!(SLABS).cast::<Slab>();
        for index in 0..MAX_SLABS {
            let slab = unsafe { slabs.add(index) };
            let active = unsafe { (*slab).active };
            let slab_page = unsafe { (*slab).page };
            if !active || slab_page != page {
                continue;
            }
            let class_index = unsafe { (*slab).class_index };
            let class_size = CLASS_SIZES[class_index];
            let offset = address - page;
            if offset % class_size != 0 {
                return;
            }
            let slot = offset / class_size;
            let slot_count = PAGE_SIZE / class_size;
            if slot >= slot_count {
                return;
            }
            let word = slot / u64::BITS as usize;
            let mask = 1u64 << (slot % u64::BITS as usize);
            let bitmap = unsafe { (*slab).bitmap[word] };
            if bitmap & mask == 0 {
                return;
            }
            unsafe {
                (*slab).bitmap[word] = bitmap & !mask;
                (*slab).used -= 1;
            }
            if unsafe { (*slab).used } == 0 && crate::memory::release(page, PAGE_SIZE) {
                let heads = addr_of_mut!(CLASS_HEADS).cast::<usize>();
                let mut previous = NO_INDEX;
                let mut current = unsafe { *heads.add(class_index) };
                while current != NO_INDEX {
                    let current_slab = unsafe { slabs.add(current) };
                    if current == index {
                        let next = unsafe { (*current_slab).next };
                        if previous == NO_INDEX {
                            unsafe { *heads.add(class_index) = next };
                        } else {
                            unsafe { (*slabs.add(previous)).next = next };
                        }
                        unsafe { slab.write(Slab::EMPTY) };
                        break;
                    }
                    previous = current;
                    current = unsafe { (*current_slab).next };
                }
            }
            return;
        }
    }

    unsafe fn allocation_capacity(&self, pointer: *const u8) -> Option<usize> {
        if pointer.is_null() || !INITIALIZED.load(Ordering::Acquire) {
            return None;
        }
        let address = pointer as usize;
        let allocations = addr_of_mut!(LARGE_ALLOCATIONS).cast::<LargeAllocation>();
        for index in 0..MAX_LARGE_ALLOCATIONS {
            let allocation = unsafe { &*allocations.add(index) };
            if allocation.active && allocation.address == address {
                return Some(allocation.length);
            }
        }

        let page = address & !(PAGE_SIZE - 1);
        let slabs = addr_of_mut!(SLABS).cast::<Slab>();
        for index in 0..MAX_SLABS {
            let slab = unsafe { &*slabs.add(index) };
            if !slab.active || slab.page != page {
                continue;
            }
            let class_size = CLASS_SIZES[slab.class_index];
            let offset = address - page;
            if offset % class_size != 0 {
                return None;
            }
            let slot = offset / class_size;
            if slot >= PAGE_SIZE / class_size {
                return None;
            }
            let mask = 1u64 << (slot % u64::BITS as usize);
            return (slab.bitmap[slot / u64::BITS as usize] & mask != 0)
                .then_some(class_size);
        }
        None
    }
}

unsafe impl GlobalAlloc for SlabAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { self.allocate(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, _layout: Layout) {
        unsafe { self.deallocate(pointer) }
    }
}

pub(crate) fn initialize() {
    let _guard = lock_heap();
    let slabs = addr_of_mut!(SLABS).cast::<Slab>();
    for index in 0..MAX_SLABS {
        unsafe { slabs.add(index).write(Slab::EMPTY) };
    }
    let heads = addr_of_mut!(CLASS_HEADS).cast::<usize>();
    for index in 0..CLASS_SIZES.len() {
        unsafe { heads.add(index).write(NO_INDEX) };
    }
    let allocations = addr_of_mut!(LARGE_ALLOCATIONS).cast::<LargeAllocation>();
    for index in 0..MAX_LARGE_ALLOCATIONS {
        unsafe { allocations.add(index).write(LargeAllocation::EMPTY) };
    }
    INITIALIZED.store(true, Ordering::Release);
}

pub(crate) fn remaining() -> usize {
    let _guard = lock_heap();
    let slabs = addr_of_mut!(SLABS).cast::<Slab>();
    let mut total = 0usize;
    for index in 0..MAX_SLABS {
        let slab = unsafe { slabs.add(index) };
        if unsafe { (*slab).active } {
            let class_size = CLASS_SIZES[unsafe { (*slab).class_index }];
            let slots = PAGE_SIZE / class_size;
            total = total.saturating_add((slots - unsafe { (*slab).used }) * class_size);
        }
    }
    total
}

fn allocate_c_layout(size: usize, alignment: usize) -> *mut c_void {
    let Ok(layout) = Layout::from_size_align(size.max(1), alignment) else {
        return null_mut();
    };
    unsafe { GLOBAL_ALLOCATOR.alloc(layout).cast::<c_void>() }
}

#[no_mangle]
pub extern "C" fn malloc(size: usize) -> *mut c_void {
    let pointer = allocate_c_layout(size, align_of::<u128>());
    if pointer.is_null() {
        unsafe { crate::kernel_thread::__errno_location().write(ENOMEM) };
    }
    pointer
}

#[no_mangle]
pub unsafe extern "C" fn aligned_alloc(alignment: usize, size: usize) -> *mut c_void {
    if alignment < align_of::<usize>() || !alignment.is_power_of_two() || size % alignment != 0 {
        unsafe { crate::kernel_thread::__errno_location().write(EINVAL) };
        return null_mut();
    }
    let pointer = allocate_c_layout(size, alignment);
    if pointer.is_null() {
        unsafe { crate::kernel_thread::__errno_location().write(ENOMEM) };
    }
    pointer
}

#[no_mangle]
pub unsafe extern "C" fn posix_memalign(
    output: *mut *mut c_void,
    alignment: usize,
    size: usize,
) -> i32 {
    if output.is_null()
        || alignment < core::mem::size_of::<*mut c_void>()
        || alignment % core::mem::size_of::<*mut c_void>() != 0
        || !alignment.is_power_of_two()
    {
        return EINVAL;
    }
    let pointer = allocate_c_layout(size, alignment);
    if pointer.is_null() {
        return ENOMEM;
    }
    unsafe { output.write(pointer) };
    0
}

#[no_mangle]
pub extern "C" fn free(pointer: *mut c_void) {
    if !pointer.is_null() {
        unsafe {
            GLOBAL_ALLOCATOR.dealloc(pointer.cast::<u8>(), Layout::new::<u8>());
        }
    }
}

#[no_mangle]
pub extern "C" fn malloc_usable_size(pointer: *mut c_void) -> usize {
    let _guard = lock_heap();
    unsafe {
        GLOBAL_ALLOCATOR
            .allocation_capacity(pointer.cast::<u8>())
            .unwrap_or(0)
    }
}

#[no_mangle]
pub unsafe extern "C" fn calloc(count: usize, size: usize) -> *mut c_void {
    let Some(total) = count.checked_mul(size) else {
        return null_mut();
    };
    let pointer = malloc(total);
    if !pointer.is_null() {
        unsafe { pointer.cast::<u8>().write_bytes(0, total) };
    }
    pointer
}

#[no_mangle]
pub unsafe extern "C" fn realloc(pointer: *mut c_void, size: usize) -> *mut c_void {
    if pointer.is_null() {
        return malloc(size);
    }
    if size == 0 {
        free(pointer);
        return null_mut();
    }

    let capacity = {
        let _guard = lock_heap();
        unsafe { GLOBAL_ALLOCATOR.allocation_capacity(pointer.cast::<u8>()) }
    };
    let Some(capacity) = capacity else {
        return null_mut();
    };
    let replacement = malloc(size);
    if replacement.is_null() {
        return null_mut();
    }
    unsafe {
        core::ptr::copy_nonoverlapping(
            pointer.cast::<u8>(),
            replacement.cast::<u8>(),
            capacity.min(size),
        );
    }
    free(pointer);
    replacement
}
