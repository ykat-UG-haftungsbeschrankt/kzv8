use crate::kernel::credentials::User;
use crate::kernel::credentials::UserId;

pub(crate) struct GroupId {
    id: u32,
}

pub(crate) struct Group {
    id: GroupId,
    name: String,
    members: HashMap<UserId, User>,
}

pub(crate) impl Group {
    pub(crate) fn new() -> Option<Self> {
        Some(Self {
            id: GroupId {
                id: 0
            },
            name: String::new(),
            members: HashMap::new(),
        })
    }
}

pub(crate) struct User {
    user_id: UserId,
    username: String,
    password: String,
}

pub(crate) impl User {
    pub(crate) fn new() -> Option<Self> {
        Some(Self {
            user_id: UserId {
                device: block::BlockDevice::initialize()?,
            },
            username: String::new(),
            password: String::new(),
        })
    }
}