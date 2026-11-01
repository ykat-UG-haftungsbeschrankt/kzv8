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

    pub(crate) fn submit(&mut self, request: Request<'_>) -> Result<(), virtio_block::Error> {
        match request.operation {
            Operation::Read(output) => self.device.read_sectors(request.sector, output),
            Operation::Write(input) => self.device.write_sectors(request.sector, input),
        }
    }
}
