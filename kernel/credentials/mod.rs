/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

//@READONLY

mod group;
mod registry;
mod user;

pub(crate) use group::Group;
#[allow(unused_imports)]
pub(crate) use registry::{user_group_registry, UserGroupRegistry};
pub(crate) use user::User;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct UserId(u32);

impl UserId {
    pub(crate) const ROOT: Self = Self(0);

    pub(crate) const fn new(id: u32) -> Self {
        Self(id)
    }

    pub(crate) const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct GroupId(u32);

impl GroupId {
    pub(crate) const ROOT: Self = Self(0);

    pub(crate) const fn new(id: u32) -> Self {
        Self(id)
    }

    pub(crate) const fn raw(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
#[path = "../../tests/kernel/credentials.rs"]
mod tests;