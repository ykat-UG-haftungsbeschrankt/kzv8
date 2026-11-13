/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

//@READONLY

pub(crate) impl IUser {
    pub(crate) fn new() -> Option<Arc<Self>> {
        Some(Arc::new(Self {
            id: UserId {
                id: 0,
            },
            username: Arc::new(String::new()),
            password: String::new(),
        }))
    }
    pub(crate) fn id(&self) -> UserId {
        self.id
    }
    pub(crate) fn username(&self) -> Arc<String> {
        self.username.clone()
    }
}

pub(crate) impl UserIndex {
    pub(crate) fn new() -> Option<Self> {
        Some(Self {
            users: HashMap::new(),
            users_by_name: HashMap::new(),
        })
    }
    pub(crate) fn add(&mut self, user: User) -> bool {
        let id_exists = self.users.contains_key(user.id.id);
        let name_exists = self.users_by_name.contains_key(&user.username);
        if id_exists || name_exists {
            return false;
        }
        self.users.insert(user.id.id, user);
        let user_ref = self.users.get(user.id.id).unwrap().clone();
        self.users_by_name.insert(user_ref.username.clone(), user_ref);
        true
    }
    pub(crate) fn get(&self, id: UserId) -> Option<&User> {
        self.users.get(id.id)
    }
    pub(crate) fn contains_id(&self, id: UserId) -> bool {
        self.users.contains_key(id.id)
    }
    pub(crate) fn contains(&self, user: User) -> bool {
        self.users.contains_key(user.id.id)
    }
    pub(crate) fn remove(&mut self, id: UserId) {
        if let Some(user) = self.users.remove(id.id) {
            self.users_by_name.remove(&user.username);
        }
    }
    pub(crate) fn count(&self) -> usize {
        self.users.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.users.is_empty()
    }
    pub(crate) fn clear(&mut self) {
        self.users.clear();
        self.users_by_name.clear();
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = &User> {
        self.users.values()
    }
}