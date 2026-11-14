/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::error::Error;
use crate::kernel::process::Credentials;
use crate::kernel::vfs::{NodeMetadata, NodeMetadataApi as _};
use alloc::sync::Arc;

#[test]
fn xattr_hash_map_preserves_create_replace_get_list_remove() {
    let metadata = NodeMetadata::new();
    let credentials = Credentials::root();
    let name = b"user.label";

    metadata.set_xattr(&credentials, name, b"first", 0).unwrap();
    let first_value = metadata.get_xattr(name).unwrap();
    assert_eq!(first_value.as_bytes(), b"first");
    let same_value = metadata.get_xattr(name).unwrap();
    assert!(Arc::ptr_eq(&first_value, &same_value));
    let listed = metadata.list_xattrs().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].as_bytes(), name);
    let listed_again = metadata.list_xattrs().unwrap();
    assert!(Arc::ptr_eq(&listed[0], &listed_again[0]));

    assert_eq!(
        metadata.set_xattr(&credentials, name, b"duplicate", super::XATTR_CREATE),
        Err(Error::Exist)
    );
    metadata
        .set_xattr(&credentials, name, b"replacement", super::XATTR_REPLACE)
        .unwrap();
    let replacement = metadata.get_xattr(name).unwrap();
    assert_eq!(replacement.as_bytes(), b"replacement");
    assert!(!Arc::ptr_eq(&first_value, &replacement));
    assert_eq!(
        metadata.set_xattr(&credentials, b"user.missing", b"value", super::XATTR_REPLACE),
        Err(Error::NoData)
    );

    metadata.remove_xattr(&credentials, name).unwrap();
    assert_eq!(metadata.get_xattr(name).map(|_| ()), Err(Error::NoData));
    assert!(metadata.list_xattrs().unwrap().is_empty());
}