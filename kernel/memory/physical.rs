use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

const PAGE_SIZE: usize = 4096;
const MAX_FRAMES: usize = 128 * 1024 * 1024 / PAGE_SIZE;
const BITMAP_WORDS: usize = MAX_FRAMES / u64::BITS as usize;

static BITMAP: [AtomicU64; BITMAP_WORDS] = [const { AtomicU64::new(u64::MAX) }; BITMAP_WORDS];
static INITIALIZED: AtomicBool = AtomicBool::new(false);
static FRAME_COUNT: AtomicUsize = AtomicUsize::new(0);
static FIRST_FREE: AtomicUsize = AtomicUsize::new(0);
static NEXT_HINT: AtomicUsize = AtomicUsize::new(0);

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
    let region = crate::platform::memory_region();
    let frame_count = (region.length / PAGE_SIZE).min(MAX_FRAMES);
    let image_end = core::ptr::addr_of!(__bss_end) as usize;
    let first_free_address =
        image_end.max(region.start).saturating_add(PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let first_free = first_free_address.saturating_sub(region.start) / PAGE_SIZE;
    if first_free >= frame_count {
        return false;
    }

    for word in &BITMAP {
        word.store(u64::MAX, Ordering::Relaxed);
    }
    for frame in first_free..frame_count {
        BITMAP[frame / u64::BITS as usize]
            .fetch_and(!(1u64 << (frame % u64::BITS as usize)), Ordering::Relaxed);
    }
    FRAME_COUNT.store(frame_count, Ordering::Relaxed);
    FIRST_FREE.store(first_free, Ordering::Relaxed);
    NEXT_HINT.store(first_free, Ordering::Relaxed);
    INITIALIZED.store(true, Ordering::Release);
    true
}

pub(crate) fn allocate() -> Option<Frame> {
    if !INITIALIZED.load(Ordering::Acquire) {
        return None;
    }
    let first = FIRST_FREE.load(Ordering::Relaxed);
    let count = FRAME_COUNT.load(Ordering::Relaxed);
    let available = count.checked_sub(first)?;
    if available == 0 {
        return None;
    }
    let hint = NEXT_HINT.load(Ordering::Relaxed).clamp(first, count - 1);

    for offset in 0..available {
        let frame = first + (hint - first + offset) % available;
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
                    NEXT_HINT.store(first + (frame + 1 - first) % available, Ordering::Relaxed);
                    let base = crate::platform::memory_region().start;
                    return Some(Frame {
                        address: base + frame * PAGE_SIZE,
                    });
                }
                Err(updated) => current = updated,
            }
        }
    }
    None
}

pub(crate) fn deallocate(frame: Frame) -> bool {
    let region = crate::platform::memory_region();
    if frame.address < region.start || (frame.address - region.start) % PAGE_SIZE != 0 {
        return false;
    }
    let index = (frame.address - region.start) / PAGE_SIZE;
    let first = FIRST_FREE.load(Ordering::Relaxed);
    let count = FRAME_COUNT.load(Ordering::Relaxed);
    if index < first || index >= count {
        return false;
    }
    let mask = 1u64 << (index % u64::BITS as usize);
    let previous = BITMAP[index / u64::BITS as usize].fetch_or(mask, Ordering::AcqRel);
    previous & mask != 0
}

pub(crate) fn free_frames() -> usize {
    let first = FIRST_FREE.load(Ordering::Relaxed);
    let count = FRAME_COUNT.load(Ordering::Relaxed);
    (first..count)
        .filter(|frame| {
            BITMAP[*frame / u64::BITS as usize].load(Ordering::Relaxed)
                & (1u64 << (*frame % u64::BITS as usize))
                == 0
        })
        .count()
}
