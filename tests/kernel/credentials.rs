/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::{GroupId, UserGroupRegistry, UserId};
use crate::kernel::error::Error;

#[test]
fn users_and_groups_are_indexed_by_id_and_name() {
    let mut registry = UserGroupRegistry::new();
    let user_id = UserId::new(1000);
    let group_id = GroupId::new(100);

    assert_eq!(registry.register_user(user_id, "alice"), Ok(()));
    assert_eq!(registry.register_group(group_id, "staff"), Ok(()));
    assert_eq!(registry.user_count(), 1);
    assert_eq!(registry.group_count(), 1);
    assert_eq!(registry.user_by_id(user_id).unwrap().username(), "alice");
    assert_eq!(registry.user_by_name("alice").unwrap().id(), user_id);
    assert_eq!(registry.group_by_id(group_id).unwrap().name(), "staff");
    assert_eq!(registry.group_by_name("staff").unwrap().id(), group_id);

    assert_eq!(
        registry.register_user(UserId::new(1000), "bob"),
        Err(Error::Exist)
    );
    assert_eq!(
        registry.register_user(UserId::new(1001), "alice"),
        Err(Error::Exist)
    );
    assert_eq!(
        registry.register_group(GroupId::new(100), "engineering"),
        Err(Error::Exist)
    );
    assert_eq!(
        registry.register_group(GroupId::new(101), "staff"),
        Err(Error::Exist)
    );
}

#[test]
fn membership_is_validated_and_group_results_are_ordered() {
    let mut registry = UserGroupRegistry::new();
    let user_id = UserId::new(1000);
    let first_group = GroupId::new(100);
    let second_group = GroupId::new(10);
    registry.register_user(user_id, "alice").unwrap();
    registry.register_group(first_group, "staff").unwrap();
    registry.register_group(second_group, "wheel").unwrap();

    assert_eq!(registry.add_member(user_id, first_group), Ok(true));
    assert_eq!(registry.add_member(user_id, first_group), Ok(false));
    assert_eq!(registry.add_member(user_id, second_group), Ok(true));
    assert!(registry.is_member(user_id, first_group));
    assert_eq!(registry.groups_for_user(user_id).unwrap(), [second_group, first_group]);

    assert_eq!(registry.remove_member(user_id, first_group), Ok(true));
    assert_eq!(registry.remove_member(user_id, first_group), Ok(false));
    assert!(!registry.is_member(user_id, first_group));
    assert_eq!(
        registry.add_member(user_id, GroupId::new(999)),
        Err(Error::NoEnt)
    );
    assert_eq!(
        registry.add_member(UserId::new(999), second_group),
        Err(Error::NoEnt)
    );
}

#[test]
fn removing_users_cleans_memberships_and_name_indexes() {
    let mut registry = UserGroupRegistry::new();
    let user_id = UserId::new(1000);
    let group_id = GroupId::new(100);
    registry.register_user(user_id, "alice").unwrap();
    registry.register_group(group_id, "staff").unwrap();
    registry.add_member(user_id, group_id).unwrap();

    let removed = registry.unregister_user(user_id).unwrap();
    assert_eq!(removed.username(), "alice");
    assert_eq!(registry.user_count(), 0);
    assert!(registry.user_by_id(user_id).is_none());
    assert!(registry.user_by_name("alice").is_none());
    assert!(!registry.is_member(user_id, group_id));
    assert_eq!(registry.groups_for_user(user_id), Err(Error::NoEnt));

    assert_eq!(registry.register_user(user_id, "alice"), Ok(()));
    let removed_group = registry.unregister_group(group_id).unwrap();
    assert_eq!(removed_group.name(), "staff");
    assert!(registry.group_by_id(group_id).is_none());
    assert!(registry.group_by_name("staff").is_none());
}

#[test]
fn empty_and_nul_names_are_rejected() {
    let mut registry = UserGroupRegistry::new();
    assert_eq!(registry.register_user(UserId::new(1), ""), Err(Error::Inval));
    assert_eq!(
        registry.register_group(GroupId::new(1), "team\0name"),
        Err(Error::Inval)
    );
}