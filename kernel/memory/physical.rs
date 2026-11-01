use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::sync::IrqSpinLock;

const PAGE_SIZE: usize = 4096;
const MAX_FRAMES: usize = crate::platform::MAX_PHYSICAL_ADDRESS / PAGE_SIZE;
const BITMAP_WORDS: usize = MAX_FRAMES / u64::BITS as usize;

static BITMAP: [AtomicU64; BITMAP_WORDS] = [const { AtomicU64::new(u64::MAX) }; BITMAP_WORDS];
static USABLE: [AtomicU64; BITMAP_WORDS] = [const { AtomicU64::new(0) }; BITMAP_WORDS];
static INITIALIZED: AtomicBool = AtomicBool::new(false);
static FRAME_COUNT: AtomicUsize = AtomicUsize::new(0);
static NEXT_HINT: AtomicUsize = AtomicUsize::new(0);
static MEMORY_BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOCATOR_LOCK: IrqSpinLock<()> = IrqSpinLock::new(());

extern "C" {
    static __bss_end: u8;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Frame {
    address: usize,
}

impl Frame {
    pub(crate) const fn address(self) -> usize {
        self.address
    }
}

pub(crate) fn initialize() -> bool {
    let _guard = ALLOCATOR_LOCK.lock();
    let image_end = core::ptr::addr_of!(__bss_end) as usize;
    for word in &BITMAP {
        word.store(u64::MAX, Ordering::Relaxed);
    }
    for word in &USABLE {
        word.store(0, Ordering::Relaxed);
    }

    let mut frame_count = 0;
    let mut total_frames = 0;
    let mut next_hint = None;
    for region in crate::platform::memory_regions() {
        let Some(region_end) = region.start.checked_add(region.length) else {
            continue;
        };
        let Some(start_address) = image_end
            .max(region.start)
            .checked_add(PAGE_SIZE - 1)
            .map(|address| address & !(PAGE_SIZE - 1))
        else {
            continue;
        };
        let end_address = region_end.min(crate::platform::MAX_PHYSICAL_ADDRESS) & !(PAGE_SIZE - 1);
        if start_address >= end_address {
            continue;
        }

        let start_frame = start_address / PAGE_SIZE;
        let end_frame = end_address / PAGE_SIZE;
        frame_count = frame_count.max(end_frame);
        next_hint = Some(next_hint.map_or(start_frame, |hint: usize| hint.min(start_frame)));
        for frame in start_frame..end_frame {
            let word_index = frame / u64::BITS as usize;
            let mask = 1u64 << (frame % u64::BITS as usize);
            let previous = USABLE[word_index].fetch_or(mask, Ordering::Relaxed);
            if previous & mask == 0 {
                BITMAP[word_index].fetch_and(!mask, Ordering::Relaxed);
                total_frames += 1;
            }
        }
    }
    if total_frames == 0 {
        return false;
    }

    FRAME_COUNT.store(frame_count, Ordering::Relaxed);
    NEXT_HINT.store(next_hint.unwrap_or(0), Ordering::Relaxed);
    MEMORY_BYTES.store(total_frames * PAGE_SIZE, Ordering::Relaxed);
    INITIALIZED.store(true, Ordering::Release);
    true
}

pub(crate) fn allocate() -> Option<Frame> {
    let _guard = ALLOCATOR_LOCK.lock();
    if !INITIALIZED.load(Ordering::Acquire) {
        return None;
    }
    let count = FRAME_COUNT.load(Ordering::Relaxed);
    if count == 0 {
        return None;
    }
    let hint = NEXT_HINT.load(Ordering::Relaxed).min(count - 1);

    for offset in 0..count {
        let frame = (hint + offset) % count;
        let word = &BITMAP[frame / u64::BITS as usize];
        let mask = 1u64 << (frame % u64::BITS as usize);
        let mut current = word.load(Ordering::Relaxed);
        loop {
            if current & mask != 0 {
                break;
            }
            match word.compare_exchange_weak(
                current,
                current | mask,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    NEXT_HINT.store(
                        if frame + 1 == count { 0 } else { frame + 1 },
                        Ordering::Relaxed,
                    );
                    return Some(Frame {
                        address: frame * PAGE_SIZE,
                    });
                }
                Err(updated) => current = updated,
            }
        }
    }
    None
}

pub(crate) fn allocate_contiguous(length: usize, alignment: usize) -> Option<usize> {
    let _guard = ALLOCATOR_LOCK.lock();
    if !INITIALIZED.load(Ordering::Acquire)
        || length == 0
        || length % PAGE_SIZE != 0
        || alignment < PAGE_SIZE
        || !alignment.is_power_of_two()
    {
        return None;
    }

    let count = FRAME_COUNT.load(Ordering::Relaxed);
    let pages = length / PAGE_SIZE;
    if count == 0 || pages > count {
        return None;
    }

    let hint = NEXT_HINT.load(Ordering::Relaxed).min(count - 1);
    for offset in 0..count {
        let frame = (hint + offset) % count;
        let Some(end_frame) = frame.checked_add(pages) else {
            continue;
        };
        if end_frame > count {
            continue;
        }
        let Some(address) = frame.checked_mul(PAGE_SIZE) else {
            continue;
        };
        if address & (alignment - 1) != 0 {
            continue;
        }

        let range_is_free = (frame..end_frame).all(|candidate| {
            let word_index = candidate / u64::BITS as usize;
            let mask = 1u64 << (candidate % u64::BITS as usize);
            USABLE[word_index].load(Ordering::Acquire) & mask != 0
                && BITMAP[word_index].load(Ordering::Acquire) & mask == 0
        });
        if !range_is_free {
            continue;
        }

        let mut reserved_end = frame;
        let mut collision = false;
        for candidate in frame..end_frame {
            let mask = 1u64 << (candidate % u64::BITS as usize);
            let previous = BITMAP[candidate / u64::BITS as usize].fetch_or(mask, Ordering::AcqRel);
            if previous & mask != 0 {
                collision = true;
                break;
            }
            reserved_end = candidate + 1;
        }

        if collision {
            for candidate in frame..reserved_end {
                let mask = 1u64 << (candidate % u64::BITS as usize);
                BITMAP[candidate / u64::BITS as usize].fetch_and(!mask, Ordering::Release);
            }
            continue;
        }

        NEXT_HINT.store(
            if end_frame == count { 0 } else { end_frame },
            Ordering::Relaxed,
        );
        return Some(address);
    }

    None
}

pub(crate) fn deallocate_contiguous(address: usize, length: usize) -> bool {
    let _guard = ALLOCATOR_LOCK.lock();
    if length == 0 || length % PAGE_SIZE != 0 || address % PAGE_SIZE != 0 {
        return false;
    }

    let count = FRAME_COUNT.load(Ordering::Relaxed);
    let start_frame = address / PAGE_SIZE;
    let pages = length / PAGE_SIZE;
    let Some(end_frame) = start_frame.checked_add(pages) else {
        return false;
    };
    if end_frame > count {
        return false;
    }

    let range_is_allocated = (start_frame..end_frame).all(|frame| {
        let word_index = frame / u64::BITS as usize;
        let mask = 1u64 << (frame % u64::BITS as usize);
        USABLE[word_index].load(Ordering::Acquire) & mask != 0
            && BITMAP[word_index].load(Ordering::Acquire) & mask != 0
    });
    if !range_is_allocated {
        return false;
    }

    for frame in start_frame..end_frame {
        let mask = 1u64 << (frame % u64::BITS as usize);
        BITMAP[frame / u64::BITS as usize].fetch_and(!mask, Ordering::Release);
    }
    NEXT_HINT.store(start_frame, Ordering::Relaxed);
    true
}

pub(crate) fn deallocate(frame: Frame) -> bool {
    let _guard = ALLOCATOR_LOCK.lock();
    if frame.address % PAGE_SIZE != 0 || frame.address >= crate::platform::MAX_PHYSICAL_ADDRESS {
        return false;
    }
    let index = frame.address / PAGE_SIZE;
    let count = FRAME_COUNT.load(Ordering::Relaxed);
    if index >= count {
        return false;
    }
    let mask = 1u64 << (index % u64::BITS as usize);
    let word_index = index / u64::BITS as usize;
    if USABLE[word_index].load(Ordering::Acquire) & mask == 0 {
        return false;
    }
    let previous = BITMAP[word_index].fetch_or(mask, Ordering::AcqRel);
    previous & mask != 0
}

pub(crate) fn free_frames() -> usize {
    let _guard = ALLOCATOR_LOCK.lock();
    let count = FRAME_COUNT.load(Ordering::Relaxed);
    (0..count)
        .filter(|frame| {
            BITMAP[*frame / u64::BITS as usize].load(Ordering::Relaxed)
                & (1u64 << (*frame % u64::BITS as usize))
                == 0
        })
        .count()
}

pub(crate) fn memory_bytes() -> usize {
    MEMORY_BYTES.load(Ordering::Acquire)
}
