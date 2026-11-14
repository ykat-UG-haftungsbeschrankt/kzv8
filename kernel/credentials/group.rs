/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

//@READONLY

use alloc::string::String;
use alloc::vec::Vec;

use crate::kernel::error::Error;
use crate::kernel::lib::hashmap::HashMap;

use super::registry::copy_name;
use super::{GroupId, UserId};

pub(crate) struct Group {
    id: GroupId,
    name: String,
    members: HashMap<u32, ()>,
}

impl Group {
    pub(super) fn new(id: GroupId, name: String) -> Self {
        Self {
            id,
            name,
            members: HashMap::new(),
        }
    }

    pub(crate) const fn id(&self) -> GroupId {
        self.id
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn is_member(&self, user_id: UserId) -> bool {
        self.members.contains_key(&user_id.raw())
    }

    pub(crate) fn member_count(&self) -> usize {
        self.members.len()
    }

    fn add_member(&mut self, user_id: UserId) -> Result<bool, Error> {
        if self.is_member(user_id) {
            return Ok(false);
        }
        match self.members.insert(user_id.raw(), ()) {
            Ok(core::option::Option::None) => Ok(true),
            Ok(core::option::Option::Some(())) => Ok(false),
            Err(_) => Err(Error::NoMem),
        }
    }

    fn remove_member(&mut self, user_id: UserId) -> bool {
        self.members.remove(&user_id.raw()).is_some()
    }
}

pub(super) struct GroupIndex {
    groups: HashMap<u32, Group>,
    groups_by_name: HashMap<String, u32>,
}

impl GroupIndex {
    pub(super) const fn new() -> Self {
        Self {
            groups: HashMap::new(),
            groups_by_name: HashMap::new(),
        }
    }

    pub(super) fn insert(&mut self, group: Group) -> Result<(), Error> {
        let id = group.id().raw();
        if self.groups.contains_key(&id) || self.groups_by_name.contains_key(group.name()) {
            return Err(Error::Exist);
        }
        let name = copy_name(group.name())?;
        match self.groups.insert(id, group) {
            Ok(core::option::Option::None) => {}
            Ok(core::option::Option::Some(_)) => return Err(Error::Exist),
            Err(_) => return Err(Error::NoMem),
        }
        match self.groups_by_name.insert(name, id) {
            Ok(core::option::Option::None) => Ok(()),
            Ok(core::option::Option::Some(_)) => {
                self.groups.remove(&id);
                Err(Error::Exist)
            }
            Err(_) => {
                self.groups.remove(&id);
                Err(Error::NoMem)
            }
        }
    }

    pub(super) fn get(&self, id: GroupId) -> Option<&Group> {
        self.groups.get(&id.raw())
    }

    pub(super) fn get_by_name(&self, name: &str) -> Option<&Group> {
        self.groups_by_name
            .get(name)
            .and_then(|id| self.groups.get(id))
    }

    pub(super) fn remove(&mut self, id: GroupId) -> Option<Group> {
        let group = self.groups.remove(&id.raw())?;
        self.groups_by_name.remove(group.name());
        Some(group)
    }

    pub(super) fn len(&self) -> usize {
        self.groups.len()
    }

    pub(super) fn add_member(
        &mut self,
        user_id: UserId,
        group_id: GroupId,
    ) -> Result<bool, Error> {
        self.groups
            .get_mut(&group_id.raw())
            .ok_or(Error::NoEnt)?
            .add_member(user_id)
    }

    pub(super) fn remove_member(
        &mut self,
        user_id: UserId,
        group_id: GroupId,
    ) -> Result<bool, Error> {
        let group = self.groups.get_mut(&group_id.raw()).ok_or(Error::NoEnt)?;
        Ok(group.remove_member(user_id))
    }

    pub(super) fn remove_member_from_all(&mut self, user_id: UserId) {
        self.groups.for_each_mut(|_, group| {
            group.remove_member(user_id);
        });
    }

    pub(super) fn groups_for_user(&self, user_id: UserId) -> Result<Vec<GroupId>, Error> {
        let mut groups = Vec::new();
        self.groups.try_for_each(|_, group| {
            if group.is_member(user_id) {
                groups.try_reserve(1).map_err(|_| Error::NoMem)?;
                groups.push(group.id());
            }
            Ok(())
        })?;
        Ok(groups)
    }
}
