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