/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

//@READONLY



pub(crate) impl TGroup for IGroup {
    fn init() -> Option<Self> {
        Some(Self {
            id: GroupId {
                id: 0
            },
            name: String::new(),
            members: HashMap::new(),
        })
    }
    fn new() -> Option<Arc<Self>> {
        Some(Arc::new(Self::init()?.unwrap()))
    }
    fn id(&self) -> GroupId {
        self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
}

pub(crate) impl GroupIndex {
    pub(crate) fn new() -> Option<Self> {
        Some(Self {
            groups: HashMap::new(),
            groups_by_name: HashMap::new(),
        })
    }
    pub(crate) fn add(&mut self, group: Group) -> bool {
        let id_exists = self.groups.contains_key(&group.id.id);
        let name_exists = self.groups_by_name.contains_key(&group.name);
        if id_exists || name_exists {
            return false;
        }
        self.groups.insert(group.id.id, group);
        let group_ref = self.groups.get(group.id.id).unwrap().clone();
        self.groups_by_name.insert(group_ref.name.clone(), group_ref);
        true
    }
    pub(crate) fn get(&self, id: GroupId) -> Option<&Group> {
        self.groups.get(id.id)
    }
    pub(crate) fn contains_id(&self, id: GroupId) -> bool {
        self.groups.contains_key(id.id)
    }
    pub(crate) fn contains(&self, group: Group) -> bool {
        self.groups.contains_key(group.id.id)
    }
    pub(crate) fn remove(&mut self, id: GroupId) {
        if let Some(group) = self.groups.remove(id.id) {
            self.groups_by_name.remove(&group.name);
        }
    }
    pub(crate) fn count(&self) -> usize {
        self.groups.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }
    pub(crate) fn clear(&mut self) {
        self.groups.clear();
        self.groups_by_name.clear();
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Group> {
        self.groups.values()
    }
}