/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::ffi::c_void;

use crate::kernel::arch::traits::MemoryArchitecture as _;
use crate::kernel::arch::MemoryArchitecture;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::scheduler::thread::ThreadManager;

use super::errno::{as_errno, EINVAL, ENOMEM};

const PAGE_SIZE: usize = 4096;

const MADV_WILLNEED: i32 = 3;
const MADV_DONTNEED: i32 = 4;
const MADV_FREE: i32 = 8;

const PROT_READ: i32 = 1;
const PROT_WRITE: i32 = 2;
const PROT_EXEC: i32 = 4;
const PROT_READ_WRITE: i32 = PROT_READ | PROT_WRITE;
const PROT_READ_EXEC: i32 = PROT_READ | PROT_EXEC;
const PROT_WRITE_EXEC: i32 = PROT_WRITE | PROT_EXEC;
const PROT_READ_WRITE_EXEC: i32 = PROT_READ | PROT_WRITE | PROT_EXEC;

const MAP_SHARED: i32 = 1;
const MAP_PRIVATE: i32 = 2;
const MAP_ANONYMOUS: i32 = 0x20;
const MAP_NORESERVE: i32 = 0x4000;
const MAP_FIXED: i32 = 0x10;

#[derive(Clone, Copy)]
struct MappingRange {
    start: usize,
    length: usize,
}

#[derive(Default)]
struct MemoryUsage {
    mappings: Vec<MappingRange>,
}

struct ProcessMemoryUsage {
    process_id: i32,
    usage: MemoryUsage,
}

#[derive(Default)]
struct MemoryAccounting {
    processes: Vec<ProcessMemoryUsage>,
}

impl MemoryAccounting {
    fn process_usage_mut(&mut self, process_id: i32) -> Result<&mut ProcessMemoryUsage, i32> {
        let index = if let Some(index) = self
            .processes
            .iter()
            .position(|process| process.process_id == process_id)
        {
            index
        } else {
            self.processes.try_reserve(1).map_err(|_| ENOMEM)?;
            self.processes.push(ProcessMemoryUsage {
                process_id,
                usage: MemoryUsage::default(),
            });
            self.processes.len() - 1
        };
        self.processes.get_mut(index).ok_or(ENOMEM)
    }
}

static MEMORY_ACCOUNTING: IrqSpinLock<MemoryAccounting> =
    IrqSpinLock::new(MemoryAccounting { processes: Vec::new() });

fn set_errno(error: i32) {
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
}

fn fail(error: i32) -> *mut c_void {
    set_errno(error);
    usize::MAX as *mut c_void
}

fn rounded_length(length: usize) -> Option<usize> {
    length
        .checked_add(PAGE_SIZE - 1)
        .map(|value| value & !(PAGE_SIZE - 1))
}

fn page_permissions(protection: i32) -> Option<i32> {
    match protection {
        0 => Some(0),
        PROT_READ => Some(1),
        PROT_WRITE | PROT_READ_WRITE => Some(2),
        PROT_EXEC | PROT_READ_EXEC => Some(4),
        PROT_WRITE_EXEC | PROT_READ_WRITE_EXEC => Some(3),
        _ => None,
    }
}

#[no_mangle]
pub unsafe extern "C" fn mmap(
    address_hint: *mut c_void,
    length: usize,
    protection: i32,
    flags: i32,
    file_descriptor: i32,
    offset: i64,
) -> *mut c_void {
    let sharing = flags & (MAP_PRIVATE | MAP_SHARED);
    let supported_flags = MAP_PRIVATE | MAP_SHARED | MAP_ANONYMOUS | MAP_NORESERVE | MAP_FIXED;
    if length == 0
        || protection & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0
        || flags & !supported_flags != 0
        || flags & MAP_ANONYMOUS == 0
        || !matches!(sharing, MAP_PRIVATE | MAP_SHARED)
        || file_descriptor != -1
        || offset != 0
    {
        return fail(EINVAL);
    }
    let Some(permissions) = page_permissions(protection) else {
        return fail(EINVAL);
    };
    let Some(length) = rounded_length(length) else {
        return fail(ENOMEM);
    };
    if flags & MAP_FIXED != 0 {
        if address_hint.is_null() || permissions != 0 {
            return fail(EINVAL);
        }
        let process = ThreadManager::current_process().ok();
        let mapping_start = address_hint as usize;
        let mapping_end = mapping_start.checked_add(length);
        let tracked_mapping = process.as_ref().is_some_and(|process| {
            let Some(mapping_end) = mapping_end else {
                return false;
            };
            MEMORY_ACCOUNTING
                .lock()
                .processes
                .iter()
                .find(|usage| usage.process_id == process.pid())
                .is_some_and(|usage| {
                    usage.usage.mappings.iter().any(|mapping| {
                        mapping.start <= mapping_start
                            && mapping
                                .start
                                .checked_add(mapping.length)
                                .is_some_and(|end| end >= mapping_end)
                    })
                })
        });
        let Some((resident_bytes, decommitted)) =
            crate::kernel::memory::r#virtual::decommit_accounted(mapping_start, length)
        else {
            return fail(ENOMEM);
        };
        if tracked_mapping {
            if let Some(process) = process {
                process.release_memory(0, 0, resident_bytes);
            }
        }
        if !decommitted {
            return fail(ENOMEM);
        }
        return address_hint;
    }
    let process = ThreadManager::current_process().ok();
    let Some(process) = process else {
        let Some(address) =
            crate::kernel::memory::r#virtual::reserve(length, PAGE_SIZE, permissions)
        else {
            return fail(ENOMEM);
        };
        return address as *mut c_void;
    };
    let Ok(length_bytes) = u64::try_from(length) else {
        return fail(ENOMEM);
    };
    let mut accounting = MEMORY_ACCOUNTING.lock();
    let usage = match accounting.process_usage_mut(process.pid()) {
        Ok(process_usage) => &mut process_usage.usage,
        Err(error) => return fail(error),
    };
    if usage.mappings.try_reserve(1).is_err() {
        return fail(ENOMEM);
    }
    if let Err(error) = process.try_reserve_memory(length_bytes) {
        return fail(as_errno(error));
    }
    let Some(address) =
        crate::kernel::memory::r#virtual::reserve(length, PAGE_SIZE, permissions)
    else {
        process.release_memory(length_bytes, length_bytes, 0);
        return fail(ENOMEM);
    };
    usage.mappings.push(MappingRange {
        start: address,
        length,
    });
    address as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn mmap64(
    address_hint: *mut c_void,
    length: usize,
    protection: i32,
    flags: i32,
    file_descriptor: i32,
    offset: i64,
) -> *mut c_void {
    unsafe {
        mmap(
            address_hint,
            length,
            protection,
            flags,
            file_descriptor,
            offset,
        )
    }
}

#[no_mangle]
pub unsafe extern "C" fn mprotect(address: *mut c_void, length: usize, protection: i32) -> i32 {
    if address.is_null() || address as usize % PAGE_SIZE != 0 || length == 0 {
        set_errno(EINVAL);
        return -1;
    }
    let Some(permissions) = page_permissions(protection) else {
        set_errno(EINVAL);
        return -1;
    };
    let address = address as usize;
    if crate::kernel::memory::r#virtual::set_permissions(address, length, permissions) {
        return 0;
    }
    if MemoryArchitecture::protect_kernel_image_pages(address, length, permissions) {
        return 0;
    }
    set_errno(ENOMEM);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn madvise(address: *mut c_void, length: usize, advice: i32) -> i32 {
    if address.is_null()
        || address as usize % PAGE_SIZE != 0
        || length == 0
        || rounded_length(length).is_none()
        || !matches!(advice, MADV_WILLNEED | MADV_DONTNEED | MADV_FREE)
    {
        set_errno(EINVAL);
        return -1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn munmap(address: *mut c_void, length: usize) -> i32 {
    let address = address as usize;
    if address == 0 || address % PAGE_SIZE != 0 || length == 0 {
        set_errno(EINVAL);
        return -1;
    }
    let Some(length) = rounded_length(length) else {
        set_errno(EINVAL);
        return -1;
    };
    let Some(end) = address.checked_add(length) else {
        set_errno(EINVAL);
        return -1;
    };
    let process = ThreadManager::current_process().ok();
    let Some(process) = process else {
        if crate::kernel::memory::r#virtual::release_range(address, length) {
            return 0;
        }
        set_errno(EINVAL);
        return -1;
    };
    let mut accounting = MEMORY_ACCOUNTING.lock();
    let Some(process_index) = accounting
        .processes
        .iter()
        .position(|usage| usage.process_id == process.pid())
    else {
        drop(accounting);
        if crate::kernel::memory::r#virtual::release_range(address, length) {
            return 0;
        }
        set_errno(EINVAL);
        return -1;
    };
    let usage = &mut accounting.processes[process_index].usage;
    let Some(mapping_index) = usage.mappings.iter().position(|mapping| {
        mapping.start <= address
            && mapping
                .start
                .checked_add(mapping.length)
                .is_some_and(|mapping_end| mapping_end >= end)
    }) else {
        drop(accounting);
        if crate::kernel::memory::r#virtual::release_range(address, length) {
            return 0;
        }
        set_errno(EINVAL);
        return -1;
    };
    let mapping = usage.mappings[mapping_index];
    let mapping_end = mapping.start + mapping.length;
    let prefix_length = address - mapping.start;
    let suffix_length = mapping_end - end;
    if prefix_length != 0 && suffix_length != 0 && usage.mappings.try_reserve(1).is_err() {
        set_errno(ENOMEM);
        return -1;
    }
    let Some((released_bytes, resident_bytes)) =
        crate::kernel::memory::r#virtual::release_range_accounted(address, length)
    else {
        set_errno(EINVAL);
        return -1;
    };
    {
        if prefix_length == 0 && suffix_length == 0 {
            usage.mappings.swap_remove(mapping_index);
        } else if prefix_length == 0 {
            usage.mappings[mapping_index] = MappingRange {
                start: end,
                length: suffix_length,
            };
        } else if suffix_length == 0 {
            usage.mappings[mapping_index].length = prefix_length;
        } else {
            usage.mappings[mapping_index].length = prefix_length;
            usage.mappings.push(MappingRange {
                start: end,
                length: suffix_length,
            });
        }
    }
    let process_usage_is_empty = accounting.processes[process_index].usage.mappings.is_empty();
    if process_usage_is_empty {
        accounting.processes.swap_remove(process_index);
    }
    drop(accounting);
    process.release_memory(released_bytes, released_bytes, resident_bytes);
    0
}
