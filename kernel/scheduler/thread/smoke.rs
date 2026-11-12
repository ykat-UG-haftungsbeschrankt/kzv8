/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::kernel::arch::traits::InterruptArchitecture as _;
use crate::kernel::dl::{AddressSpace, Error, Permissions};
use super::vm::UserAddressSpace;
use super::{ThreadManager, ThreadStack, ThreadState};

static THREAD_SMOKE_RESULT: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn thread_smoke_worker(argument: *mut c_void) -> *mut c_void {
    THREAD_SMOKE_RESULT.store(argument as usize, Ordering::Release);
    argument
}

pub(crate) fn thread_smoke() -> bool {
    const RESULT: usize = 0x4b5a;

    THREAD_SMOKE_RESULT.store(0, Ordering::Release);
    let Ok(thread_id) = ThreadManager::create_thread(
        ThreadStack::MIN_SIZE,
        thread_smoke_worker,
        RESULT as *mut c_void,
        false,
    ) else {
        return false;
    };
    let interrupt_state =
        crate::kernel::arch::InterruptArchitecture::disable_interrupts();
    let join_result = match ThreadManager::begin_join_by_id(thread_id) {
        Ok(Some(result)) => Ok(result),
        Ok(None) if ThreadManager::switch_current(ThreadState::Blocked) => {
            ThreadManager::finish_join_by_id(thread_id)
        }
        Ok(None) => {
            ThreadManager::cancel_join_by_id(thread_id);
            Err(Error::Deadlk)
        }
        Err(error) => Err(error),
    };
    crate::kernel::arch::InterruptArchitecture::restore_interrupts(interrupt_state);

    matches!(join_result, Ok(RESULT))
        && THREAD_SMOKE_RESULT.load(Ordering::Acquire) == RESULT
        && !ThreadManager::contains_thread_by_id(thread_id)
}

pub(crate) fn vm_smoke() -> bool {
    const PAGE_SIZE: usize = crate::kernel::memory::r#virtual::PAGE_SIZE;
    const MAPPING_LENGTH: usize = PAGE_SIZE * 2;

    let Ok(process) = ThreadManager::current_process() else {
        return false;
    };
    let mut address_space = process.address_space();
    if !matches!(
        address_space.reserve(None, 0, PAGE_SIZE, 0, false),
        Err(Error::NoExec)
    ) {
        return false;
    }

    let Ok(stack_address) = address_space.reserve(None, PAGE_SIZE + 1, PAGE_SIZE, 0, false) else {
        return false;
    };
    let Some(stack_pointer) = stack_address.checked_add(MAPPING_LENGTH) else {
        address_space.release(stack_address, MAPPING_LENGTH);
        return false;
    };
    let read_write = Permissions::from_bits(Permissions::READ.bits() | Permissions::WRITE.bits());
    let stack_result = (|| -> Result<(), Error> {
        if stack_address % PAGE_SIZE != 0
            || stack_address.checked_add(MAPPING_LENGTH) != Some(stack_pointer)
        {
            return Err(Error::Inval);
        }
        address_space.protect(stack_address, MAPPING_LENGTH, read_write)?;
        let mut bytes = [u8::MAX; 4];
        address_space.read(stack_address, &mut bytes)?;
        if bytes != [0; 4] {
            return Err(Error::Inval);
        }
        Ok(())
    })();
    address_space.release(stack_address, MAPPING_LENGTH);
    if stack_result.is_err() {
        return false;
    }
    let Ok(reclaimed_stack) =
        address_space.reserve(Some(stack_address), MAPPING_LENGTH, PAGE_SIZE, 0, true)
    else {
        return false;
    };
    address_space.release(reclaimed_stack, MAPPING_LENGTH);

    let Ok(address) = address_space.reserve(
        None,
        PAGE_SIZE + 1,
        MAPPING_LENGTH,
        PAGE_SIZE,
        false,
    ) else {
        return false;
    };
    if address % MAPPING_LENGTH != PAGE_SIZE {
        address_space.release(address, MAPPING_LENGTH);
        return false;
    }

    let mapping_result = (|| -> Result<(), Error> {
        address_space.zero(address, MAPPING_LENGTH)?;
        address_space.protect(address, MAPPING_LENGTH, read_write)?;
        let mut bytes = [u8::MAX; 4];
        address_space.read(address, &mut bytes)?;
        if bytes != [0; 4] {
            return Err(Error::Inval);
        }

        let expected = [0x5a, 0xa5, 0x3c, 0xc3];
        address_space.write(address, &expected)?;
        address_space.read(address, &mut bytes)?;
        if bytes != expected {
            return Err(Error::Inval);
        }
        address_space.protect(address, MAPPING_LENGTH, Permissions::READ)?;
        if !matches!(address_space.write(address, &expected), Err(Error::NoMem)) {
            return Err(Error::Inval);
        }
        address_space.read(address, &mut bytes)?;
        if bytes != expected {
            return Err(Error::Inval);
        }
        Ok(())
    })();
    address_space.release(address, MAPPING_LENGTH);
    if mapping_result.is_err() {
        return false;
    }
    let Ok(reclaimed_mapping) =
        address_space.reserve(Some(address), MAPPING_LENGTH, PAGE_SIZE, 0, true)
    else {
        return false;
    };
    address_space.release(reclaimed_mapping, MAPPING_LENGTH);
    address_space_isolation_smoke(&mut address_space)
}

fn address_space_isolation_smoke(address_space: &mut UserAddressSpace) -> bool {
    const PAGE_SIZE: usize = crate::kernel::memory::r#virtual::PAGE_SIZE;

    let Ok(mut isolated_address_space) = UserAddressSpace::new() else {
        return false;
    };
    let Ok(address) = address_space.reserve(None, PAGE_SIZE, PAGE_SIZE, 0, false) else {
        return false;
    };
    let Ok(isolated_address) = isolated_address_space.reserve(
        Some(address),
        PAGE_SIZE,
        PAGE_SIZE,
        0,
        true,
    ) else {
        address_space.release(address, PAGE_SIZE);
        return false;
    };
    let result = (|| -> Result<(), Error> {
        if isolated_address != address {
            return Err(Error::Inval);
        }
        address_space.zero(address, PAGE_SIZE)?;
        let expected = [0x31, 0x72, 0xa4, 0xe8];
        address_space.write(address, &expected)?;
        isolated_address_space.zero(isolated_address, PAGE_SIZE)?;

        let mut bytes = [u8::MAX; 4];
        isolated_address_space.read(isolated_address, &mut bytes)?;
        if bytes != [0; 4] {
            return Err(Error::Inval);
        }
        address_space.read(address, &mut bytes)?;
        if bytes != expected {
            return Err(Error::Inval);
        }

        if !isolated_address_space.release_user_mapping(isolated_address, PAGE_SIZE) {
            return Err(Error::NoMem);
        }
        address_space.read(address, &mut bytes)?;
        if bytes != expected {
            return Err(Error::Inval);
        }
        Ok(())
    })();
    address_space.release(address, PAGE_SIZE);
    result.is_ok()
}