/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::block::{
    BlockClient, BlockDevice, BlockDeviceApi as _, BlockService, BlockServiceApi as _,
};
use crate::kernel::net::stack::NetStack;

pub(crate) struct KernelVirtioServices {
    pub(crate) block_device: Option<BlockClient>,
    pub(crate) network_stack: Option<NetStack>,
}

impl KernelVirtioServices {
    pub(crate) fn initialize() -> Self {
        Self {
            block_device: BlockDevice::initialize().and_then(BlockService::start),
            network_stack: NetStack::initialize(),
        }
    }
}