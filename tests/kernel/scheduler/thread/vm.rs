/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;

use crate::kernel::scheduler::sync::IrqSpinLock;
use super::{UserAddressSpace, UserAddressSpaceInner, UserAreas};

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
