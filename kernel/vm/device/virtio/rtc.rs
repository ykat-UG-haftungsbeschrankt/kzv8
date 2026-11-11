/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::arch::traits::TimerArchitecture as _;
use crate::kernel::vm::device::virtio::mmio::{QueueResult, VirtioMmio};
use crate::kernel::vm::device::virtio::queue::GuestVirtQueue;
use crate::kernel::vm::device::GuestMemory;

pub(crate) const MMIO_BASE: u64 = 0x1000_1000;
const DEVICE_ID: u32 = 42;
const VENDOR_ID: u32 = 0x4b5a;
const CLOCK_COUNT: u16 = 1;
const MESSAGE_HEADER_SIZE: usize = 8;
const MESSAGE_BUFFER_SIZE: usize = 64;

const REQ_READ: u16 = 0x0001;
const REQ_READ_CROSS: u16 = 0x0002;
const REQ_CFG: u16 = 0x1000;
const REQ_CLOCK_CAP: u16 = 0x1001;
const REQ_CROSS_CAP: u16 = 0x1002;

const RESP_OK: u8 = 0;
const RESP_NOT_SUPPORTED: u8 = 2;
const RESP_NO_DEVICE: u8 = 3;
const RESP_INVALID: u8 = 4;

const CLOCK_MONOTONIC: u8 = 2;

pub(crate) struct VirtioRtc {
    mmio: VirtioMmio,
}

impl VirtioRtc {
    pub(crate) const fn new() -> Self {
        Self {
            mmio: VirtioMmio::new(
                MMIO_BASE,
                DEVICE_ID,
                VENDOR_ID,
                [0, 1],
                [0; 16],
                [0; 16],
                1,
                256,
            ),
        }
    }

    pub(crate) const fn contains(address: u64) -> bool {
        address >= MMIO_BASE && address < MMIO_BASE + 0x1000
    }

    pub(crate) fn access(
        &mut self,
        memory: &mut GuestMemory<'_>,
        address: u64,
        width: u8,
        write: bool,
        value: u64,
    ) -> Option<u64> {
        self.mmio
            .access(memory, address, width, write, value, Self::notify_queue)
    }

    fn notify_queue(
        queue_index: usize,
        queue: &mut GuestVirtQueue,
        memory: &mut GuestMemory<'_>,
    ) -> QueueResult {
        if queue_index != 0 {
            return QueueResult::Invalid;
        }
        let mut completed = false;
        loop {
            let chain = match queue.available_chain(memory) {
                Ok(Some(chain)) => chain,
                Ok(None) => break,
                Err(()) => return QueueResult::Invalid,
            };
            let mut request = [0; MESSAGE_BUFFER_SIZE];
            let Some(request_length) = chain.read_device_data(memory, &mut request) else {
                return QueueResult::Invalid;
            };
            let mut response = [0; MESSAGE_BUFFER_SIZE];
            let Some(response_length) = Self::handle_request(&request[..request_length], &mut response)
            else {
                return QueueResult::Invalid;
            };
            let Some(used_length) =
                chain.write_device_data(memory, &response[..response_length])
            else {
                return QueueResult::Invalid;
            };
            if !queue.complete(memory, &chain, used_length) {
                return QueueResult::Invalid;
            }
            completed = true;
        }
        if completed {
            QueueResult::Complete
        } else {
            QueueResult::Pending
        }
    }

    fn handle_request(request: &[u8], response: &mut [u8; MESSAGE_BUFFER_SIZE]) -> Option<usize> {
        if request.len() < MESSAGE_HEADER_SIZE {
            return None;
        }
        response.fill(0);
        let request_type = u16::from_le_bytes([request[0], request[1]]);
        let response_length = match request_type {
            REQ_CFG => {
                if request.len() < MESSAGE_HEADER_SIZE {
                    return None;
                }
                response[MESSAGE_HEADER_SIZE..MESSAGE_HEADER_SIZE + 2]
                    .copy_from_slice(&CLOCK_COUNT.to_le_bytes());
                16
            }
            REQ_CLOCK_CAP => {
                if request.len() < 16 {
                    response[0] = RESP_INVALID;
                } else if Self::request_clock_id(request)? >= CLOCK_COUNT {
                    response[0] = RESP_NO_DEVICE;
                } else {
                    response[MESSAGE_HEADER_SIZE] = CLOCK_MONOTONIC;
                }
                16
            }
            REQ_READ => {
                if request.len() < 16 {
                    response[0] = RESP_INVALID;
                } else if Self::request_clock_id(request)? >= CLOCK_COUNT {
                    response[0] = RESP_NO_DEVICE;
                } else {
                    let Some(now_ns) = crate::kernel::arch::TimerArchitecture::now_ns() else {
                        response[0] = RESP_NOT_SUPPORTED;
                        return Some(16);
                    };
                    response[MESSAGE_HEADER_SIZE..MESSAGE_HEADER_SIZE + 8]
                        .copy_from_slice(&now_ns.to_le_bytes());
                }
                16
            }
            REQ_CROSS_CAP => {
                if request.len() < 16 {
                    response[0] = RESP_INVALID;
                } else if Self::request_clock_id(request)? >= CLOCK_COUNT {
                    response[0] = RESP_NO_DEVICE;
                } else if request[MESSAGE_HEADER_SIZE + 2] > 1 {
                    response[0] = RESP_INVALID;
                }
                16
            }
            REQ_READ_CROSS => {
                response[0] = if request.len() < 16 {
                    RESP_INVALID
                } else {
                    let clock_id = Self::request_clock_id(request)?;
                    if clock_id >= CLOCK_COUNT {
                        RESP_NO_DEVICE
                    } else if request[MESSAGE_HEADER_SIZE + 2] > 1 {
                        RESP_INVALID
                    } else {
                        RESP_NOT_SUPPORTED
                    }
                };
                24
            }
            _ => {
                response[0] = RESP_NOT_SUPPORTED;
                16
            }
        };
        Some(response_length)
    }

    fn request_clock_id(request: &[u8]) -> Option<u16> {
        if request.len() < MESSAGE_HEADER_SIZE + 2 {
            return None;
        }
        Some(u16::from_le_bytes([
            request[MESSAGE_HEADER_SIZE],
            request[MESSAGE_HEADER_SIZE + 1],
        ]))
    }
}