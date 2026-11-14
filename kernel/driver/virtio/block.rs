/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use crate::kernel::driver::virtio::queue::{Descriptor, DmaQueue};
use crate::kernel::driver::virtio::transport::VirtioTransport;
use crate::kernel::error::Error;
use crate::kernel::memory::dma::{DmaArena, DmaBuffer};
use crate::kernel::scheduler::sync::IrqSpinLock;

pub(crate) struct BlockDevice {
    transport: VirtioTransport,
    capacity_sectors: u64,
    state: IrqSpinLock<BlockState>,
}

struct BlockState {
    queue: DmaQueue<{ BlockDevice::QUEUE_SIZE }>,
    //@FIXME: INDEXTYPE(requests); small direct-index request pool; Vec lookup is O(1), keep Vec.
    requests: Vec<BlockRequest>,
    //@FIXME: INDEXTYPE(free_requests); LIFO free-index stack; Vec push/pop are amortized O(1), keep Vec.
    free_requests: Vec<usize>,
    request_by_head: [Option<usize>; BlockDevice::QUEUE_SIZE],
}

struct BlockRequest {
    dma_buffer: DmaBuffer,
    head: Option<u16>,
    used_length: Option<u32>,
    abandoned: bool,
}

impl BlockDevice {
    const DEVICE_ID: u32 = 2;
    const QUEUE_SIZE: usize = 8;
    const SECTOR_SIZE: usize = 512;
    const MAX_TRANSFER_SIZE: usize = 64 * 1024;
    const HEADER_SIZE: usize = 16;
    const STATUS_OFFSET: usize = Self::HEADER_SIZE;
    const DATA_OFFSET: usize = 32;

    pub(crate) fn initialize() -> Option<Self> {
        let candidate = VirtioTransport::find(Self::DEVICE_ID)?;
        let transport = candidate.initialize(Self::DEVICE_ID).ok()?;
        let mut queue = DmaQueue::new()?;
        queue.configure_with_min_size(&transport, 0, 3).ok()?;
        if !transport.activate() {
            return None;
        }
        let capacity_sectors = u64::from(transport.read_device_config(0)?)
            | (u64::from(transport.read_device_config(4)?) << 32);
        let mut requests = Vec::new();
        let mut free_requests = Vec::new();
        let request_count = Self::QUEUE_SIZE / 3;
        requests.try_reserve_exact(request_count).ok()?;
        free_requests.try_reserve_exact(request_count).ok()?;
        for request_index in 0..request_count {
            requests.push(BlockRequest {
                dma_buffer: DmaArena::allocate(
                    Self::DATA_OFFSET + Self::MAX_TRANSFER_SIZE,
                    16,
                )?,
                head: None,
                used_length: None,
                abandoned: false,
            });
            free_requests.push(request_count - request_index - 1);
        }
        Some(Self {
            transport,
            capacity_sectors,
            state: IrqSpinLock::new(BlockState {
                queue,
                requests,
                free_requests,
                request_by_head: [None; Self::QUEUE_SIZE],
            }),
        })
    }

    pub(crate) const fn capacity_sectors(&self) -> u64 {
        self.capacity_sectors
    }

    pub(crate) fn submit_read<'a>(
        &'a self,
        sector: u64,
        output: &'a mut [u8],
    ) -> Result<BlockTransfer<'a>, Error> {
        self.validate_request(sector, output.len())?;
        self.submit(0, sector, output.len(), Some(output), None)
    }

    pub(crate) fn submit_write<'a>(
        &'a self,
        sector: u64,
        input: &[u8],
    ) -> Result<BlockTransfer<'a>, Error> {
        self.validate_request(sector, input.len())?;
        self.submit(1, sector, input.len(), None, Some(input))
    }

    fn validate_request(&self, sector: u64, length: usize) -> Result<(), Error> {
        if length == 0 || length % Self::SECTOR_SIZE != 0 || length > Self::MAX_TRANSFER_SIZE {
            return Err(Error::Inval);
        }
        if sector
            .checked_add((length / Self::SECTOR_SIZE) as u64)
            .is_none_or(|end| end > self.capacity_sectors)
        {
            return Err(Error::Inval);
        }
        Ok(())
    }

    fn submit<'a>(
        &'a self,
        request_type: u32,
        sector: u64,
        length: usize,
        output: Option<&'a mut [u8]>,
        input: Option<&[u8]>,
    ) -> Result<BlockTransfer<'a>, Error> {
        let mut state = self.state.lock();
        Self::reap_completions(&mut state)?;
        let Some(request_index) = state.free_requests.pop() else {
            return Err(Error::Busy);
        };
        let request = &mut state.requests[request_index];
        let bus_address = request.dma_buffer.bus_address();
        let buffer = request.dma_buffer.as_mut_slice();
        buffer[..4].copy_from_slice(&request_type.to_le_bytes());
        buffer[4..8].fill(0);
        buffer[8..Self::HEADER_SIZE].copy_from_slice(&sector.to_le_bytes());
        buffer[Self::STATUS_OFFSET] = 0xff;
        if let Some(input) = input {
            buffer[Self::DATA_OFFSET..Self::DATA_OFFSET + length].copy_from_slice(input);
        }
        let descriptors = [
            Descriptor::new(
                bus_address,
                Self::HEADER_SIZE as u32,
                false,
            ),
            Descriptor::new(
                bus_address + Self::DATA_OFFSET as u64,
                length as u32,
                request_type == 0,
            ),
            Descriptor::new(bus_address + Self::STATUS_OFFSET as u64, 1, true),
        ];
        let Some(head) = state.queue.submit(&descriptors) else {
            state.free_requests.push(request_index);
            return Err(Error::Busy);
        };
        state.request_by_head[usize::from(head)] = Some(request_index);
        let request = &mut state.requests[request_index];
        request.head = Some(head);
        request.abandoned = false;
        drop(state);
        DmaArena::sync_for_device();
        self.transport.notify(0);
        Ok(BlockTransfer {
            device: self,
            output,
            length,
            request_index,
            head,
            complete: false,
        })
    }

    fn reap_completions(state: &mut BlockState) -> Result<(), Error> {
        while let Some(used) = state.queue.poll_used() {
            let Some(request_index) = state
                .request_by_head
                .get_mut(used.id as usize)
                .and_then(Option::take)
            else {
                return Err(Error::Proto);
            };
            let abandoned = {
                let Some(request) = state.requests.get_mut(request_index) else {
                    return Err(Error::Proto);
                };
                if request.head.map(u32::from) != Some(used.id) {
                    return Err(Error::Proto);
                }
                if request.abandoned {
                    request.head = None;
                    request.used_length = None;
                    request.abandoned = false;
                    true
                } else {
                    request.used_length = Some(used.length);
                    false
                }
            };
            if abandoned {
                state.free_requests.push(request_index);
            }
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn interrupt_slot(&self) -> usize {
        self.transport.slot()
    }
}

pub(crate) struct BlockTransfer<'a> {
    device: &'a BlockDevice,
    output: Option<&'a mut [u8]>,
    length: usize,
    request_index: usize,
    head: u16,
    complete: bool,
}

impl BlockTransfer<'_> {
    pub(crate) fn poll(&mut self) -> Result<bool, Error> {
        if self.complete {
            return Ok(true);
        }
        let mut state = self.device.state.lock();
        BlockDevice::reap_completions(&mut state)?;
        let request = state
            .requests
            .get_mut(self.request_index)
            .ok_or(Error::Proto)?;
        if request.head != Some(self.head) {
            return Err(Error::Proto);
        }
        let completion_result = {
            let Some(used_length) = request.used_length.take() else {
                return Ok(false);
            };
            DmaArena::sync_for_cpu();
            self.complete = true;
            request.head = None;
            request.abandoned = false;
            let status = request.dma_buffer.as_mut_slice()[BlockDevice::STATUS_OFFSET];
            match status {
                0 if used_length >= 1 => {
                    if let Some(output) = self.output.as_mut() {
                        let source = &request.dma_buffer.as_mut_slice()[
                            BlockDevice::DATA_OFFSET..BlockDevice::DATA_OFFSET + self.length
                        ];
                        (**output).copy_from_slice(source);
                    }
                    Ok(())
                }
                0 => Err(Error::Proto),
                1 => Err(Error::Io),
                2 => Err(Error::OpNotSupp),
                _ => Err(Error::Proto),
            }
        };
        state.free_requests.push(self.request_index);
        completion_result?;
        Ok(true)
    }
}

impl Drop for BlockTransfer<'_> {
    fn drop(&mut self) {
        if self.complete {
            return;
        }
        let mut state = self.device.state.lock();
        let release_request = {
            let Some(request) = state.requests.get_mut(self.request_index) else {
                return;
            };
            if request.head == Some(self.head) {
                request.abandoned = true;
                if request.used_length.is_some() {
                    request.head = None;
                    request.used_length = None;
                    request.abandoned = false;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };
        if release_request {
            state.free_requests.push(self.request_index);
        }
    }
}
