/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::driver::virtio::transport::VirtioTransport;
use crate::kernel::error::Error;
use crate::kernel::memory::dma::{DmaArena, DmaBuffer};
use crate::kernel::scheduler::sync::{IrqSpinGuard, IrqSpinLock};

const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Descriptor {
    pub(crate) address: u64,
    pub(crate) length: u32,
    pub(crate) flags: u16,
    pub(crate) next: u16,
}

impl Descriptor {
    pub(crate) const fn new(address: u64, length: u32, device_writes: bool) -> Self {
        Self {
            address,
            length,
            flags: if device_writes { DESC_WRITE } else { 0 },
            next: 0,
        }
    }
}

#[repr(C, align(4096))]
struct DescriptorTable<const N: usize>([Descriptor; N]);

#[repr(C, align(4096))]
struct AvailableRing<const N: usize> {
    flags: u16,
    index: u16,
    ring: [u16; N],
    used_event: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct UsedElement {
    pub(crate) id: u32,
    pub(crate) length: u32,
}

#[repr(C, align(4096))]
struct UsedRing<const N: usize> {
    flags: u16,
    index: u16,
    ring: [UsedElement; N],
    available_event: u16,
}

pub(crate) struct VirtQueue<const N: usize> {
    descriptors: DescriptorTable<N>,
    available: AvailableRing<N>,
    used: UsedRing<N>,
    free_descriptors: [bool; N],
    free_descriptor_stack: [u16; N],
    free_descriptor_count: usize,
    pending_heads: [bool; N],
    last_used: u16,
    queue_size: usize,
}

impl<const N: usize> VirtQueue<N> {
    pub(crate) const fn new() -> Self {
        assert!(N > 0 && N <= u16::MAX as usize);
        Self {
            descriptors: DescriptorTable(
                [Descriptor {
                    address: 0,
                    length: 0,
                    flags: 0,
                    next: 0,
                }; N],
            ),
            available: AvailableRing {
                flags: 0,
                index: 0,
                ring: [0; N],
                used_event: 0,
            },
            used: UsedRing {
                flags: 0,
                ring: [UsedElement { id: 0, length: 0 }; N],
                available_event: 0,
            },
            free_descriptors: [false; N],
            free_descriptor_stack: [0; N],
            free_descriptor_count: 0,
            pending_heads: [false; N],
            in_flight_count: 0,
            last_used: 0,
            queue_size: 0,
        }
    }

    pub(crate) fn configure(
        &mut self,
        transport: &VirtioTransport,
        index: u32,
    ) -> Result<(), Error> {
        self.configure_with_min_size(transport, index, 1)
    }

    pub(crate) fn configure_with_min_size(
        &mut self,
        transport: &VirtioTransport,
        index: u32,
        minimum_size: usize,
    ) -> Result<(), Error> {
        if minimum_size == 0 || minimum_size > N {
            return Err(Error::Inval);
        }
        let queue_size = transport.configure_queue(
            index,
            N as u32,
            minimum_size as u32,
            core::ptr::addr_of_mut!(self.descriptors.0) as u64,
            core::ptr::addr_of_mut!(self.available) as u64,
            core::ptr::addr_of_mut!(self.used) as u64,
        )?;
        self.queue_size = queue_size as usize;
        self.free_descriptors = [false; N];
        for slot in 0..self.queue_size {
            self.free_descriptors[slot] = true;
            self.free_descriptor_stack[slot] = (self.queue_size - slot - 1) as u16;
        }
        self.free_descriptor_count = self.queue_size;
        self.pending_heads = [false; N];
        self.last_used = 0;
        Ok(())
    }

    pub(crate) fn submit(&mut self, chain: &[Descriptor]) -> Option<u16> {
        let queue_size = self.queue_size;
        if queue_size == 0 || chain.is_empty() || chain.len() > queue_size {
            return None;
        }
        if self.free_descriptor_count < chain.len() {
            return None;
        }

        let mut slots = [0; N];
        for offset in 0..chain.len() {
            slots[offset] = self.free_descriptor_stack[self.free_descriptor_count - offset - 1];
        }
        self.free_descriptor_count -= chain.len();

        for slot in slots[..chain.len()].iter().copied() {
            self.free_descriptors[usize::from(slot)] = false;
        }
        let head = slots[0];
        for (offset, descriptor) in chain.iter().copied().enumerate() {
            let slot = usize::from(slots[offset]);
            let mut descriptor = descriptor;
            if offset + 1 < chain.len() {
                descriptor.flags |= DESC_NEXT;
                descriptor.next = slots[offset + 1];
            } else {
                descriptor.flags &= !DESC_NEXT;
                descriptor.next = 0;
            }
            unsafe {
                core::ptr::write_volatile(
                    core::ptr::addr_of_mut!(self.descriptors.0[slot]),
                    descriptor,
                );
            }
        }

        let available_index =
            unsafe { core::ptr::read_volatile(core::ptr::addr_of!(self.available.index)) };
        let ring_slot = usize::from(available_index) % queue_size;
        unsafe {
            core::ptr::write_volatile(
                core::ptr::addr_of_mut!(self.available.ring[ring_slot]),
                head,
            );
        }
        DmaArena::sync_for_device();
        unsafe {
            core::ptr::write_volatile(
                core::ptr::addr_of_mut!(self.available.index),
                available_index.wrapping_add(1),
            );
        }
        self.pending_heads[usize::from(head)] = true;
        Some(head)
    }

    pub(crate) fn poll_used(&mut self) -> Option<UsedElement> {
        let queue_size = self.queue_size;
        if queue_size == 0 {
            return None;
        }
        let used_index = unsafe { core::ptr::read_volatile(core::ptr::addr_of!(self.used.index)) };
        if self.last_used == used_index {
            return None;
        }
        DmaArena::sync_for_cpu();
        let slot = usize::from(self.last_used) % queue_size;
        let element =
            unsafe { core::ptr::read_volatile(core::ptr::addr_of!(self.used.ring[slot])) };
        self.last_used = self.last_used.wrapping_add(1);
        let head = element.id as usize;
        if head < queue_size && self.pending_heads[head] {
            self.pending_heads[head] = false;
            let mut descriptor_slot = head;
            for _ in 0..queue_size {
                if self.free_descriptors[descriptor_slot] {
                    break;
                }
                let descriptor = unsafe {
                    core::ptr::read_volatile(core::ptr::addr_of!(
                        self.descriptors.0[descriptor_slot]
                    ))
                };
                self.free_descriptors[descriptor_slot] = true;
                self.free_descriptor_stack[self.free_descriptor_count] = descriptor_slot as u16;
                self.free_descriptor_count += 1;
                if descriptor.flags & DESC_NEXT == 0 {
                    break;
                }
                let next = usize::from(descriptor.next);
                if next >= queue_size {
                    break;
                }
                descriptor_slot = next;
            }
        }
        Some(element)
    }

}

pub(crate) fn wait_used(
    mut poll: impl FnMut() -> Option<UsedElement>,
    spins: usize,
) -> Option<UsedElement> {
    for _ in 0..spins {
        if let Some(element) = poll() {
            return Some(element);
        }
        core::hint::spin_loop();
    }
    None
}

pub(crate) struct DmaQueue<const N: usize> {
    storage: DmaBuffer,
}

impl<const N: usize> DmaQueue<N> {
    pub(crate) fn new() -> Option<Self> {
        let mut storage = DmaArena::allocate(
            core::mem::size_of::<VirtQueue<N>>(),
            core::mem::align_of::<VirtQueue<N>>(),
        )?;
        let queue = storage.as_mut_slice().as_mut_ptr().cast::<VirtQueue<N>>();
        // SAFETY: storage has the size and alignment of VirtQueue<N> and is exclusively owned.
        unsafe { queue.write(VirtQueue::new()) };
        Some(Self { storage })
    }

    fn with_queue<R>(&mut self, operation: impl for<'a> FnOnce(&'a mut VirtQueue<N>) -> R) -> R {
        let queue = self
            .storage
            .as_mut_slice()
            .as_mut_ptr()
            .cast::<VirtQueue<N>>();
        // SAFETY: new initialized this uniquely owned DMA allocation as VirtQueue<N>.
        unsafe { operation(&mut *queue) }
    }

    pub(crate) fn configure(
        &mut self,
        transport: &VirtioTransport,
        index: u32,
    ) -> Result<(), Error> {
        self.with_queue(|queue| queue.configure(transport, index))
    }

    pub(crate) fn configure_with_min_size(
        &mut self,
        transport: &VirtioTransport,
        index: u32,
        minimum_size: usize,
    ) -> Result<(), Error> {
        self.with_queue(|queue| queue.configure_with_min_size(transport, index, minimum_size))
    }

    pub(crate) fn submit(&mut self, chain: &[Descriptor]) -> Option<u16> {
        self.with_queue(|queue| queue.submit(chain))
    }

    pub(crate) fn poll_used(&mut self) -> Option<UsedElement> {
        self.with_queue(VirtQueue::poll_used)
    }

}

pub(crate) struct QueueCell<const N: usize> {
    queue: IrqSpinLock<VirtQueue<N>>,
}

impl<const N: usize> QueueCell<N> {
    pub(crate) const fn new() -> Self {
        Self {
            queue: IrqSpinLock::new(VirtQueue::new()),
        }
    }

    pub(crate) fn lock(&self) -> IrqSpinGuard<'_, VirtQueue<N>> {
        self.queue.lock()
    }
}
