/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct IrqSpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

// SAFETY: all access to data is serialized by locked; lock() also masks local
// interrupts, and T: Send permits ownership to move between CPUs.
unsafe impl<T: Send> Sync for IrqSpinLock<T> {}

impl<T> IrqSpinLock<T> {
    pub(crate) const fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    pub(crate) fn lock(&self) -> IrqSpinGuard<'_, T> {
        let interrupt_state = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::
            InterruptArchitecture>::disable_interrupts();
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        IrqSpinGuard {
            lock: self,
            interrupt_state,
            not_send: PhantomData,
        }
    }
}

pub(crate) struct IrqSpinGuard<'a, T> {
    lock: &'a IrqSpinLock<T>,
    interrupt_state: u64,
    not_send: PhantomData<*mut ()>,
}

impl<T> Deref for IrqSpinGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: this guard keeps the lock held for the returned borrow.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for IrqSpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: this is the unique mutable guard while the lock is held.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for IrqSpinGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::InterruptArchitecture>::
            restore_interrupts(self.interrupt_state);
    }
}
