/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::driver::virtio::block;
use crate::kernel::error::Error;

use super::{BlockDeviceApi, Operation, Request};

pub(crate) struct BlockDevice {
    device: block::BlockDevice,
}

impl BlockDeviceApi for BlockDevice {
    fn initialize() -> Option<Self> {
        Some(Self {
            device: block::BlockDevice::initialize()?,
        })
    }

    fn capacity_sectors(&self) -> u64 {
        self.device.capacity_sectors()
    }

    fn interrupt_slot(&self) -> usize {
        self.device.interrupt_slot()
    }

    fn submit<'a>(
        &'a self,
        request: Request<'a>,
    ) -> Result<block::BlockTransfer<'a>, Error> {
        match request.operation {
            Operation::Read(output) => self.device.submit_read(request.sector, output),
            Operation::Write(input) => self.device.submit_write(request.sector, input),
        }
    }
}
