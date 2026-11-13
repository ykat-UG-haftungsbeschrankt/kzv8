/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

//@READONLY

use std::collections::HashMap;
use std::sync::Arc;

mod user;
mod group;

pub(crate) struct UserId {
    id: usize,
}

pub(crate) struct IUser {
    id: UserId,
    username: Arc<String>,
    password: String,
}

type User = Arc<IUser>;

pub(crate) struct UserIndex {
    users: HashMap<usize, User>,
    users_by_name: HashMap<Arc<String>, User>,
}

pub(crate) struct GroupId {
    id: u32,
}

pub(crate) struct IGroup {
    id: GroupId,
    name: String,
    members: HashMap<UserId, User>,
}

trait TGroup {
    fn init() -> Option<Self>;
    fn new() -> Option<Arc<Self>>;
    fn id(&self) -> GroupId;
    fn name(&self) -> &str;
}

type Group = Arc<IGroup>;

pub(crate) struct GroupIndex {
    groups: HashMap<usize, Group>,
    groups_by_name: HashMap<Arc<String>, Group>,
}