#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MemoryRegion {
    pub(crate) start: usize,
    pub(crate) length: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VirtioWindow {
    pub(crate) start: usize,
    pub(crate) slots: usize,
    pub(crate) stride: usize,
}

#[cfg(target_arch = "x86_64")]
pub(crate) const fn memory_region() -> MemoryRegion {
    MemoryRegion {
        start: 0,
        length: 128 * 1024 * 1024,
    }
}

#[cfg(target_arch = "aarch64")]
pub(crate) const fn memory_region() -> MemoryRegion {
    MemoryRegion {
        start: 0x4000_0000,
        length: 128 * 1024 * 1024,
    }
}

#[cfg(target_arch = "x86_64")]
pub(crate) const fn virtio_window() -> VirtioWindow {
    VirtioWindow {
        start: 0xfeb0_0000,
        slots: 24,
        stride: 0x200,
    }
}

#[cfg(target_arch = "aarch64")]
pub(crate) const fn virtio_window() -> VirtioWindow {
    VirtioWindow {
        start: 0x0a00_0000,
        slots: 32,
        stride: 0x200,
    }
}

pub(crate) unsafe fn find_virtio_device(device_id: u32) -> Option<usize> {
    let window = virtio_window();
    for slot in 0..window.slots {
        let base = window.start + slot * window.stride;
        let magic = core::ptr::read_volatile(base as *const u32);
        let version = core::ptr::read_volatile((base + 4) as *const u32);
        let id = core::ptr::read_volatile((base + 8) as *const u32);
        if magic == 0x7472_6976 && version == 2 && id == device_id {
            return Some(base);
        }
    }
    None
}
