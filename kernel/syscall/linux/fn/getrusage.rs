/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::super::SyscallFrame;
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::process::ResourceUsageSnapshot;
use crate::kernel::scheduler::thread::ThreadManager;

const RUSAGE_SELF: i32 = 0;
const RUSAGE_CHILDREN: i32 = -1;
const RUSAGE_THREAD: i32 = 1;
const RUSAGE_BOTH: i32 = -2;
const RUSAGE_LENGTH: usize = 18 * core::mem::size_of::<i64>();

pub(crate) struct Getrusage;

impl Getrusage {
    pub(crate) const ID: u64 = crate::kernel::arch::syscall::Getrusage;

    pub(crate) fn handle(frame: SyscallFrame) -> Result<i64, Error> {
        let output_address = usize::try_from(frame.arguments[1]).map_err(|_| Error::Fault)?;
        if output_address == 0 {
            return Err(Error::Fault);
        }
        let process = ThreadManager::current_process()?;
        let usage = match frame.arguments[0] as i32 {
            RUSAGE_SELF => ThreadManager::current_process_resource_usage_snapshot()
                .unwrap_or_else(|| process.resource_usage_snapshot()),
            RUSAGE_CHILDREN => process.child_resource_usage_snapshot(),
            RUSAGE_THREAD => ThreadManager::current_thread_resource_usage_snapshot()
                .ok_or(Error::Again)?,
            RUSAGE_BOTH => combine_usage(
                ThreadManager::current_process_resource_usage_snapshot()
                    .unwrap_or_else(|| process.resource_usage_snapshot()),
                process.child_resource_usage_snapshot(),
            ),
            _ => return Err(Error::Inval),
        };
        let output = encode_usage(usage)?;
        let mut address_space = process.address_space();
        address_space
            .write(output_address, &output)
            .map_err(|_| Error::Fault)?;
        Ok(0)
    }
}

fn timeval(cpu_time_ns: u64) -> Result<(i64, i64), Error> {
    let seconds = i64::try_from(cpu_time_ns / 1_000_000_000).map_err(|_| Error::Overflow)?;
    let microseconds = ((cpu_time_ns % 1_000_000_000) / 1000) as i64;
    Ok((seconds, microseconds))
}

fn counter(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn combine_usage(mut usage: ResourceUsageSnapshot, children: ResourceUsageSnapshot) -> ResourceUsageSnapshot {
    usage.user_cpu_time_ns = usage.user_cpu_time_ns.saturating_add(children.user_cpu_time_ns);
    usage.system_cpu_time_ns = usage.system_cpu_time_ns.saturating_add(children.system_cpu_time_ns);
    usage.max_rss_bytes = usage.max_rss_bytes.max(children.max_rss_bytes);
    usage.minor_page_faults = usage.minor_page_faults.saturating_add(children.minor_page_faults);
    usage.major_page_faults = usage.major_page_faults.saturating_add(children.major_page_faults);
    usage.swaps = usage.swaps.saturating_add(children.swaps);
    usage.input_blocks = usage.input_blocks.saturating_add(children.input_blocks);
    usage.output_blocks = usage.output_blocks.saturating_add(children.output_blocks);
    usage.messages_sent = usage.messages_sent.saturating_add(children.messages_sent);
    usage.messages_received = usage.messages_received.saturating_add(children.messages_received);
    usage.signals_received = usage.signals_received.saturating_add(children.signals_received);
    usage.voluntary_context_switches = usage.voluntary_context_switches.saturating_add(children.voluntary_context_switches);
    usage.involuntary_context_switches = usage.involuntary_context_switches.saturating_add(children.involuntary_context_switches);
    usage
}

pub(crate) fn encode_usage(usage: ResourceUsageSnapshot) -> Result<[u8; RUSAGE_LENGTH], Error> {
    let (user_seconds, user_microseconds) = timeval(usage.user_cpu_time_ns)?;
    let (system_seconds, system_microseconds) = timeval(usage.system_cpu_time_ns)?;
    let values = [
        user_seconds,
        user_microseconds,
        system_seconds,
        system_microseconds,
        counter(usage.max_rss_bytes / 1024),
        0,
        0,
        0,
        counter(usage.minor_page_faults),
        counter(usage.major_page_faults),
        counter(usage.swaps),
        counter(usage.input_blocks),
        counter(usage.output_blocks),
        counter(usage.messages_sent),
        counter(usage.messages_received),
        counter(usage.signals_received),
        counter(usage.voluntary_context_switches),
        counter(usage.involuntary_context_switches),
    ];
    let mut output = [0; RUSAGE_LENGTH];
    for (index, value) in values.iter().copied().enumerate() {
        let start = index * core::mem::size_of::<i64>();
        output[start..start + 8].copy_from_slice(&value.to_ne_bytes());
    }
    Ok(output)
}

#[cfg(test)]
#[path = "../../../../tests/kernel/syscall/linux/fn/getrusage.rs"]
mod tests;