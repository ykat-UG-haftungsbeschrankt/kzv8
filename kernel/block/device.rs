/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::block_request::{Operation, Request};
use crate::virtio_block;

pub(crate) struct BlockDevice {
    device: virtio_block::BlockDevice,
}

impl BlockDevice {
    pub(crate) fn initialize() -> Option<Self> {
        Some(Self {
            device: virtio_block::BlockDevice::initialize()?,
        })
    }

    pub(crate) fn capacity_sectors(&self) -> u64 {
        self.device.capacity_sectors()
    }

    pub(crate) fn interrupt_slot(&self) -> usize {
        self.device.interrupt_slot()
    }

    pub(crate) fn submit<'a>(
        &'a mut self,
        request: Request<'a>,
    ) -> Result<virtio_block::BlockTransfer<'a>, virtio_block::Error> {
        match request.operation {
            Operation::Read(output) => self.device.submit_read(request.sector, output),
            Operation::Write(input) => self.device.submit_write(request.sector, input),
        }
    }
}
