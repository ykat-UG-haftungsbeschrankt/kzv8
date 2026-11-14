/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[test]
fn pci_identity_classifies_absent_functions() {
    assert!(!super::identity_is_present(0));
    assert!(!super::identity_is_present(u32::MAX));
    assert!(super::identity_is_present(0x5678_1234));
}
