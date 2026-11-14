/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

//@READONLY

use alloc::string::String;
use alloc::sync::Arc;

use crate::kernel::error::Error;
use crate::kernel::lib::hashmap::HashMap;

use super::registry::copy_name;
use super::UserId;

pub(crate) struct User {
    id: UserId,
    username: String,
}

impl User {
    pub(super) fn new(id: UserId, username: String) -> Self {
        Self { id, username }
    }

    pub(crate) const fn id(&self) -> UserId {
        self.id
    }

    pub(crate) fn username(&self) -> &str {
        &self.username
    }
}

pub(super) struct UserIndex {
    users: HashMap<u32, Arc<User>>,
    users_by_name: HashMap<String, u32>,
}

impl UserIndex {
    pub(super) const fn new() -> Self {
        Self {
            users: HashMap::new(),
            users_by_name: HashMap::new(),
        }
    }

    pub(super) fn insert(&mut self, user: User) -> Result<(), Error> {
        self.insert_shared(Arc::new(user))
    }

    pub(super) fn insert_shared(&mut self, user: Arc<User>) -> Result<(), Error> {
        let id = user.id().raw();
        if self.users.contains_key(&id) || self.users_by_name.contains_key(user.username()) {
            return Err(Error::Exist);
        }
        let username = copy_name(user.username())?;
        match self.users.insert(id, Arc::clone(&user)) {
            Ok(core::option::Option::None) => {}
            Ok(core::option::Option::Some(_)) => return Err(Error::Exist),
            Err(_) => return Err(Error::NoMem),
        }
        match self.users_by_name.insert(username, id) {
            Ok(core::option::Option::None) => Ok(()),
            Ok(core::option::Option::Some(_)) => {
                self.users.remove(&id);
                Err(Error::Exist)
            }
            Err(_) => {
                self.users.remove(&id);
                Err(Error::NoMem)
            }
        }
    }

    pub(super) fn get(&self, id: UserId) -> Option<&User> {
        self.users.get(&id.raw()).map(Arc::as_ref)
    }

    pub(super) fn get_by_name(&self, username: &str) -> Option<&User> {
        self.users_by_name
            .get(username)
            .and_then(|id| self.users.get(id))
            .map(Arc::as_ref)
    }

    pub(super) fn get_shared(&self, id: UserId) -> Option<Arc<User>> {
        self.users.get(&id.raw()).cloned()
    }

    pub(super) fn remove(&mut self, id: UserId) -> Option<Arc<User>> {
        let user = self.users.remove(&id.raw())?;
        self.users_by_name.remove(user.username());
        Some(user)
    }

    pub(super) fn len(&self) -> usize {
        self.users.len()
    }
}