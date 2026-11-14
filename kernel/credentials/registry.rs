/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::string::String;
use alloc::vec::Vec;

use crate::kernel::error::Error;
use crate::kernel::scheduler::sync::{IrqSpinGuard, IrqSpinLock};

use super::group::GroupIndex;
use super::user::UserIndex;
use super::{Group, GroupId, User, UserId};

pub(crate) struct UserGroupRegistry {
    users: UserIndex,
    groups: GroupIndex,
}

impl UserGroupRegistry {
    pub(crate) const fn new() -> Self {
        Self {
            users: UserIndex::new(),
            groups: GroupIndex::new(),
        }
    }

    pub(crate) fn register_user(&mut self, id: UserId, username: &str) -> Result<(), Error> {
        self.users.insert(User::new(id, copy_name(username)?))
    }

    pub(crate) fn unregister_user(&mut self, id: UserId) -> Option<User> {
        let user = self.users.remove(id)?;
        self.groups.remove_member_from_all(id);
        Some(user)
    }

    pub(crate) fn user_by_id(&self, id: UserId) -> Option<&User> {
        self.users.get(id)
    }

    pub(crate) fn user_by_name(&self, username: &str) -> Option<&User> {
        self.users.get_by_name(username)
    }

    pub(crate) fn user_count(&self) -> usize {
        self.users.len()
    }

    pub(crate) fn register_group(&mut self, id: GroupId, name: &str) -> Result<(), Error> {
        self.groups.insert(Group::new(id, copy_name(name)?))
    }

    pub(crate) fn unregister_group(&mut self, id: GroupId) -> Option<Group> {
        self.groups.remove(id)
    }

    pub(crate) fn group_by_id(&self, id: GroupId) -> Option<&Group> {
        self.groups.get(id)
    }

    pub(crate) fn group_by_name(&self, name: &str) -> Option<&Group> {
        self.groups.get_by_name(name)
    }

    pub(crate) fn group_count(&self) -> usize {
        self.groups.len()
    }

    pub(crate) fn add_member(
        &mut self,
        user_id: UserId,
        group_id: GroupId,
    ) -> Result<bool, Error> {
        if self.users.get(user_id).is_none() {
            return Err(Error::NoEnt);
        }
        self.groups.add_member(user_id, group_id)
    }

    pub(crate) fn remove_member(
        &mut self,
        user_id: UserId,
        group_id: GroupId,
    ) -> Result<bool, Error> {
        if self.users.get(user_id).is_none() {
            return Err(Error::NoEnt);
        }
        self.groups.remove_member(user_id, group_id)
    }

    pub(crate) fn is_member(&self, user_id: UserId, group_id: GroupId) -> bool {
        self.groups
            .get(group_id)
            .is_some_and(|group| group.is_member(user_id))
    }

    pub(crate) fn groups_for_user(&self, user_id: UserId) -> Result<Vec<GroupId>, Error> {
        if self.users.get(user_id).is_none() {
            return Err(Error::NoEnt);
        }
        let mut groups = self.groups.groups_for_user(user_id)?;
        groups.sort_by_key(|group| group.raw());
        Ok(groups)
    }
}

static USER_GROUP_REGISTRY: IrqSpinLock<UserGroupRegistry> =
    IrqSpinLock::new(UserGroupRegistry::new());

pub(crate) fn user_group_registry() -> IrqSpinGuard<'static, UserGroupRegistry> {
    USER_GROUP_REGISTRY.lock()
}

pub(super) fn copy_name(name: &str) -> Result<String, Error> {
    if name.is_empty() || name.as_bytes().contains(&0) {
        return Err(Error::Inval);
    }
    let mut owned = String::new();
    owned
        .try_reserve_exact(name.len())
        .map_err(|_| Error::NoMem)?;
    owned.push_str(name);
    Ok(owned)
}