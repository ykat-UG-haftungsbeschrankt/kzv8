/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::arch::x86_64::__cpuid;
use core::ptr::write_bytes;

use crate::kernel::arch::traits::MemoryArchitecture as _;
use crate::kernel::arch::MemoryArchitecture;
use crate::kernel::memory::physical::PhysicalAllocator;
use crate::kernel::scheduler::sync::IrqSpinLock;

const PAGE_SIZE: usize = 4096;
const IA32_EFER: u32 = 0xc000_0080;
const IA32_VM_CR: u32 = 0xc001_0114;
const IA32_VM_HSAVE_PA: u32 = 0xc001_0117;
const EFER_SVME: u64 = 1 << 12;
const VM_CR_SVMDIS: u64 = 1 << 4;

struct SvmHostCpu {
    apic_id: u32,
    _host_save_physical: usize,
}

//@FIXME: INDEXTYPE(HOST_CPUS); initialization-time CPU list with linear APIC-ID dedup; use the kernel HashMap<u32, ()> as a side index for expected O(1) membership if it grows.
static HOST_CPUS: IrqSpinLock<Vec<SvmHostCpu>> = IrqSpinLock::new(Vec::new());

pub(crate) fn initialize_host() -> Result<(), super::super::VirtualizationError> {
    let maximum_extended_leaf = __cpuid(0x8000_0000).eax;
    if maximum_extended_leaf < 0x8000_000a
        || __cpuid(0x8000_0001).ecx & (1 << 2) == 0
        || __cpuid(0x8000_000a).edx & 1 == 0
    {
        return Err(super::super::VirtualizationError::Unsupported);
    }

    let apic_id = super::interrupts::current_apic_id();
    let mut cpus = HOST_CPUS.lock();
    if cpus.iter().any(|cpu| cpu.apic_id == apic_id) {
        return Ok(());
    }
    cpus.try_reserve(1)
        .map_err(|_| super::super::VirtualizationError::ResourceExhausted)?;

    if read_msr(IA32_VM_CR) & VM_CR_SVMDIS != 0 {
        return Err(super::super::VirtualizationError::Unsupported);
    }
    let efer = read_msr(IA32_EFER);
    if efer & EFER_SVME != 0 || read_msr(IA32_VM_HSAVE_PA) != 0 {
        return Err(super::super::VirtualizationError::HostSetupFailed);
    }

    let host_save_physical = PhysicalAllocator::allocate_contiguous(PAGE_SIZE, PAGE_SIZE)
        .ok_or(super::super::VirtualizationError::ResourceExhausted)?;
    let physical_end = host_save_physical
        .checked_add(PAGE_SIZE)
        .ok_or(super::super::VirtualizationError::HostSetupFailed)?;
    if physical_end > MemoryArchitecture::physical_address_limit() as usize
        || physical_end > MemoryArchitecture::identity_mapped_limit() as usize
        || !MemoryArchitecture::is_identity_mapped(host_save_physical)
        || !MemoryArchitecture::is_identity_mapped(physical_end - 1)
    {
        let _released = PhysicalAllocator::deallocate_contiguous(host_save_physical, PAGE_SIZE);
        return Err(super::super::VirtualizationError::Unsupported);
    }
    unsafe { write_bytes(host_save_physical as *mut u8, 0, PAGE_SIZE) };

    unsafe {
        write_msr(IA32_VM_HSAVE_PA, host_save_physical as u64);
        write_msr(IA32_EFER, efer | EFER_SVME);
    }
    cpus.push(SvmHostCpu {
        apic_id,
        _host_save_physical: host_save_physical,
    });
    Ok(())
}

fn read_msr(index: u32) -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") index,
            out("eax") low,
            out("edx") high,
            options(nostack, preserves_flags),
        );
    }
    (u64::from(high) << 32) | u64::from(low)
}

unsafe fn write_msr(index: u32, value: u64) {
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") index,
            in("eax") value as u32,
            in("edx") (value >> 32) as u32,
            options(nostack, preserves_flags),
        );
    }
}