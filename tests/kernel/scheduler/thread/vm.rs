/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;

use crate::kernel::scheduler::sync::IrqSpinLock;
use super::{UserAddressSpace, UserAddressSpaceInner, UserAreas};
#[cfg(feature = "virtualization")]
use super::{PassthroughControl, PciFunction};

impl UserAddressSpace {
    pub(crate) fn new_test_stub() -> Self {
        Self {
            inner: Arc::new(UserAddressSpaceInner {
                root: 0,
                base: 0,
                length: 0,
                areas: IrqSpinLock::new(UserAreas::new()),
                test_stub: true,
            }),
        }
    }
}

#[test]
fn user_areas_tree_reserves_gaps_and_rejects_overlaps() {
    let mut areas = UserAreas::new();
    let bounds = 0..(16 * super::USER_VM_PAGE_SIZE);

    let first = areas
        .reserve(
            bounds.clone(),
            None,
            2 * super::USER_VM_PAGE_SIZE,
            super::USER_VM_PAGE_SIZE,
            2,
        )
        .unwrap();
    let second = areas
        .reserve(
            bounds.clone(),
            None,
            2 * super::USER_VM_PAGE_SIZE,
            super::USER_VM_PAGE_SIZE,
            2,
        )
        .unwrap();

    assert_eq!(first, 0);
    assert_eq!(second, 2 * super::USER_VM_PAGE_SIZE);
    assert!(areas.is_reserved(&(0..super::USER_VM_PAGE_SIZE)));
    assert!(areas.is_reserved(
        &(2 * super::USER_VM_PAGE_SIZE..3 * super::USER_VM_PAGE_SIZE)
    ));
    assert!(!areas.is_reserved(
        &(4 * super::USER_VM_PAGE_SIZE..5 * super::USER_VM_PAGE_SIZE)
    ));
    assert!(areas
        .reserve(
            bounds,
            Some(super::USER_VM_PAGE_SIZE),
            2 * super::USER_VM_PAGE_SIZE,
            super::USER_VM_PAGE_SIZE,
            2,
        )
        .is_none());
}

#[cfg(feature = "virtualization")]
#[test]
fn unplug_ticket_completes_when_vcpu_processes_request() {
    let control = PassthroughControl::new();
    let function = PciFunction {
        segment: 0,
        bus: 0,
        device: 3,
        function: 0,
    };
    let ticket = control.request(function).unwrap();
    let request = control.take_request().unwrap();

    assert_eq!(request.function, function);
    assert_eq!(ticket.try_result(), None);
    *request.result.lock() = Some(Ok(()));
    assert_eq!(ticket.try_result(), Some(Ok(())));
}

#[cfg(feature = "virtualization")]
#[test]
fn closing_vcpu_control_completes_pending_tickets() {
    let control = PassthroughControl::new();
    let function = PciFunction {
        segment: 0,
        bus: 0,
        device: 4,
        function: 0,
    };
    let ticket = control.request(function).unwrap();
    control.close(crate::kernel::arch::VirtualizationError::HostSetupFailed);

    assert_eq!(
        ticket.try_result(),
        Some(Err(crate::kernel::arch::VirtualizationError::HostSetupFailed))
    );
    assert!(control.request(function).is_err());
}
