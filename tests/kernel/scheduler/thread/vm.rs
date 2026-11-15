/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::sync::Arc;

use crate::kernel::scheduler::sync::IrqSpinLock;
use super::{
    fork_page_needs_temporary_access, UserAddressSpace, UserAddressSpaceInner, UserAreas,
};

#[test]
fn fork_temporarily_accesses_backed_nonpresent_pages() {
    assert!(fork_page_needs_temporary_access(false, true));
    assert!(!fork_page_needs_temporary_access(true, true));
    assert!(!fork_page_needs_temporary_access(false, false));
}

impl UserAddressSpace {
    pub(crate) fn new_test_stub() -> Self {
        Self {
            inner: Arc::new(UserAddressSpaceInner {
                root: 0,
                base: 0,
                length: 0,
                areas: IrqSpinLock::new(UserAreas::new()),
                program_break: IrqSpinLock::new((0, 0, 0)),
                test_stub: true,
            }),
        }
    }
}