/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::kernel::arch::traits::{CpuArchitecture as _, TimerArchitecture as _};
use crate::kernel::block::device::BlockDevice;
use crate::kernel::block::request::Request;
use crate::kernel::error::Error;
use crate::kernel::ipc::{Capability, CapabilityRights, Endpoint, MessageBody, SharedMemory};
use crate::kernel::process::{CapabilityHandle, Process};
use crate::kernel::scheduler::thread::{ThreadManager, ThreadState};

const BLOCK_QUEUE_CAPACITY: usize = 8;
const BLOCK_SERVER_STACK_SIZE: usize = 32 * 1024;
const BLOCK_READ: u32 = 1;
const BLOCK_WRITE: u32 = 2;
const BLOCK_REPLY: u32 = 3;
const BLOCK_REQUEST_SIZE: usize = 12;
const BLOCK_MAX_TRANSFER_SIZE: usize = 64 * 1024;
const BLOCK_TIMEOUT_NS: u64 = 5_000_000_000;
const BLOCK_WAIT_ITERATIONS: usize = 10_000_000;

pub(crate) struct BlockClient {
    process: Arc<Process>,
    endpoint: CapabilityHandle,
    shared_memory: CapabilityHandle,
    capacity_sectors: u64,
    request_active: AtomicBool,
}

impl BlockClient {
    pub(crate) fn capacity_sectors(&self) -> u64 {
        self.capacity_sectors
    }

    pub(crate) fn smoke(&self) -> Result<(), Error> {
        if self.capacity_sectors == 0 {
            return Err(Error::NoDev);
        }
        let mut first = [0; 512];
        let mut second = [0; 512];
        self.read(0, &mut first)?;
        self.read(0, &mut second)?;
        if first != second {
            return Err(Error::Io);
        }
        Ok(())
    }

    pub(crate) fn read(&self, sector: u64, output: &mut [u8]) -> Result<(), Error> {
        let _active = self.begin_request()?;
        let grant = self.shared_grant()?;
        let response = self.transact(BLOCK_READ, sector, output.len())?;
        if response.opcode() != BLOCK_REPLY {
            return Err(Error::Proto);
        }
        grant.read(0, output)
    }

    pub(crate) fn write(&self, sector: u64, input: &[u8]) -> Result<(), Error> {
        let _active = self.begin_request()?;
        let grant = self.shared_grant()?;
        grant.write(0, input)?;
        let response = self.transact(BLOCK_WRITE, sector, input.len())?;
        if response.opcode() != BLOCK_REPLY {
            return Err(Error::Proto);
        }
        Ok(())
    }

    fn begin_request(&self) -> Result<BlockRequestGuard<'_>, Error> {
        self.request_active
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| Error::Busy)?;
        Ok(BlockRequestGuard(&self.request_active))
    }

    fn shared_grant(&self) -> Result<crate::kernel::ipc::SharedMemoryGrant, Error> {
        match self.process.capability(self.shared_memory)? {
            Capability::SharedMemory(grant) => Ok(grant),
            Capability::Endpoint(_) => Err(Error::Badf),
        }
    }

    fn transact(
        &self,
        operation: u32,
        sector: u64,
        length: usize,
    ) -> Result<MessageBody, Error> {
        if length == 0
            || length > BLOCK_MAX_TRANSFER_SIZE
            || length % 512 != 0
            || sector
                .checked_add((length / 512) as u64)
                .is_none_or(|end| end > self.capacity_sectors)
        {
            return Err(Error::Inval);
        }
        let mut payload = [0; BLOCK_REQUEST_SIZE];
        payload[..8].copy_from_slice(&sector.to_le_bytes());
        let length = u32::try_from(length).map_err(|_| Error::MsgSize)?;
        payload[8..].copy_from_slice(&length.to_le_bytes());
        let mut body = MessageBody::new(operation, &payload)?;
        let rights = match operation {
            BLOCK_READ => CapabilityRights::READ
                .union(CapabilityRights::WRITE)
                .union(CapabilityRights::TRANSFER),
            BLOCK_WRITE => CapabilityRights::READ.union(CapabilityRights::TRANSFER),
            _ => return Err(Error::Inval),
        };
        self.process
            .attach_capability(&mut body, self.shared_memory, rights)?;
        let ticket = self.process.call_now(self.endpoint, body)?;
        let started = block_time_ns();
        for _ in 0..BLOCK_WAIT_ITERATIONS {
            if let Some(response) = ticket.try_reply()? {
                return Ok(response);
            }
            if block_time_ns().saturating_sub(started) >= BLOCK_TIMEOUT_NS {
                return Err(Error::TimedOut);
            }
            if !ThreadManager::switch_current(ThreadState::Ready) {
                crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
            }
        }
        Err(Error::TimedOut)
    }
}

struct BlockRequestGuard<'a>(&'a AtomicBool);

impl Drop for BlockRequestGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct BlockWorker {
    process: Arc<Process>,
    receive_endpoint: CapabilityHandle,
    device: BlockDevice,
}

pub(crate) struct BlockService;

impl BlockService {
    pub(crate) fn start(device: BlockDevice) -> Option<BlockClient> {
        let process = ThreadManager::current_process().ok()?;
        let server_process = Process::new().ok()?;
        let (sender, receiver) = Endpoint::channel(BLOCK_QUEUE_CAPACITY).ok()?;
        let client_endpoint = process
            .insert_capability(Capability::Endpoint(sender))
            .ok()?;
        let shared_memory = SharedMemory::new(BLOCK_MAX_TRANSFER_SIZE).ok()?;
        let shared_grant = shared_memory
            .grant(
                0..shared_memory.len(),
                CapabilityRights::READ
                    .union(CapabilityRights::WRITE)
                    .union(CapabilityRights::TRANSFER),
            )
            .ok()?;
        let shared_memory_handle = process
            .insert_capability(Capability::SharedMemory(shared_grant))
            .ok()?;
        let receive_endpoint = server_process
            .insert_capability(Capability::Endpoint(receiver))
            .ok()?;
        let capacity_sectors = device.capacity_sectors();
        let worker = Box::new(BlockWorker {
            process: Arc::clone(&server_process),
            receive_endpoint,
            device,
        });
        let worker_pointer = Box::into_raw(worker);
        if ThreadManager::create_thread_in_process(
            server_process.clone(),
            BLOCK_SERVER_STACK_SIZE,
            block_server_entry,
            worker_pointer.cast::<c_void>(),
            true,
        )
        .is_err()
        {
            unsafe { drop(Box::from_raw(worker_pointer)) };
            let _ = process.close_capability(client_endpoint);
            let _ = process.close_capability(shared_memory_handle);
            let _ = server_process.close_capability(receive_endpoint);
            return None;
        }
        Some(BlockClient {
            process,
            endpoint: client_endpoint,
            shared_memory: shared_memory_handle,
            capacity_sectors,
            request_active: AtomicBool::new(false),
        })
    }

    pub(crate) fn ready_marker(client: &Option<BlockClient>) -> &'static str {
        if client.is_some() {
            "kz: block IPC service ready\n"
        } else {
            ""
        }
    }
}

unsafe extern "C" fn block_server_entry(argument: *mut c_void) -> *mut c_void {
    if argument.is_null() {
        ThreadManager::exit_current(core::ptr::null_mut());
    }
    // SAFETY: BlockService::start passes a Box pointer and transfers ownership to this thread.
    let mut worker = unsafe { Box::from_raw(argument.cast::<BlockWorker>()) };
    loop {
        match worker.process.try_receive(worker.receive_endpoint) {
            Ok(message) => {
                let result = serve_block_request(&mut worker.device, &worker.process, &message);
                let response = result.map(|()| {
                    MessageBody::new(BLOCK_REPLY, &[])
                        .unwrap_or_else(|_| unreachable!("empty block response is valid"))
                });
                let _replied = message.reply(response);
            }
            Err(Error::Again) => {
                if !ThreadManager::switch_current(ThreadState::Ready) {
                    crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
                }
            }
            Err(_) => ThreadManager::exit_current(core::ptr::null_mut()),
        }
    }
}

fn serve_block_request(
    device: &mut BlockDevice,
    process: &Process,
    message: &crate::kernel::process::ProcessMessage,
) -> Result<(), Error> {
    let operation = message.opcode();
    let payload = message.payload();
    if payload.len() != BLOCK_REQUEST_SIZE || !matches!(operation, BLOCK_READ | BLOCK_WRITE) {
        return Err(Error::Inval);
    }
    let sector = u64::from_le_bytes(payload[..8].try_into().map_err(|_| Error::Inval)?);
    let length = u32::from_le_bytes(payload[8..].try_into().map_err(|_| Error::Inval)?) as usize;
    if length == 0 || length > BLOCK_MAX_TRANSFER_SIZE || length % 512 != 0 {
        return Err(Error::Inval);
    }
    let grant_handle = *message.capabilities().first().ok_or(Error::Badf)?;
    let mapping = process.map_shared_memory(grant_handle)?;
    if mapping.len() < length {
        return Err(Error::Fault);
    }
    let result = match operation {
        BLOCK_READ => {
            // SAFETY: the received grant was mapped writable and remains alive for this transfer.
            let buffer = unsafe {
                core::slice::from_raw_parts_mut(mapping.address() as *mut u8, length)
            };
            let mut transfer = device.submit(Request::read(sector, buffer))?;
            wait_for_transfer(&mut transfer)
        }
        BLOCK_WRITE => {
            // SAFETY: the received grant was mapped readable and remains alive for this transfer.
            let buffer = unsafe { core::slice::from_raw_parts(mapping.address() as *const u8, length) };
            let mut transfer = device.submit(Request::write(sector, buffer))?;
            wait_for_transfer(&mut transfer)
        }
        _ => Err(Error::Inval),
    };
    drop(mapping);
    let close_result = process.close_capability(grant_handle);
    result.and(close_result)
}

fn wait_for_transfer(transfer: &mut crate::kernel::driver::virtio::block::BlockTransfer<'_>) -> Result<(), Error> {
    let started = block_time_ns();
    for _ in 0..BLOCK_WAIT_ITERATIONS {
        match transfer.poll()? {
            true => return Ok(()),
            false => {
                if block_time_ns().saturating_sub(started) >= BLOCK_TIMEOUT_NS {
                    return Err(Error::TimedOut);
                }
                crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
            }
        }
    }
    Err(Error::TimedOut)
}

fn block_time_ns() -> u64 {
    crate::kernel::arch::TimerArchitecture::now_ns()
        .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns)
}