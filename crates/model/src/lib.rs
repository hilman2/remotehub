//! Domain types of remotehub and [`Catalog::authorize`] — the only place that
//! decides permissions (ADR 0005).
//!
//! Objects live in a folder tree: folders contain folders, devices and
//! credentials. A grant gives a principal (the SID of a user or a group) a
//! role on one object. Roles are ordered, `list < connect < reveal < edit <
//! manage`; a grant on a folder holds for everything below it; grants only
//! ever allow. A person's effective role on an object is the highest role any
//! of their SIDs holds on the object or one of its folders. Administrators
//! hold `manage` everywhere.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a grant allows, from least to most. Each role includes the ones
/// before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// See that the object exists.
    List,
    /// Open a session with a device, or let a device use a credential —
    /// without seeing the password.
    Connect,
    /// See a credential's password.
    Reveal,
    /// Create, change and delete devices and credentials.
    Edit,
    /// Everything, including folders and grants.
    Manage,
}

impl Role {
    pub const ALL: [Role; 5] = [
        Role::List,
        Role::Connect,
        Role::Reveal,
        Role::Edit,
        Role::Manage,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::List => "list",
            Role::Connect => "connect",
            Role::Reveal => "reveal",
            Role::Edit => "edit",
            Role::Manage => "manage",
        }
    }

    pub fn parse(value: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| r.as_str() == value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum ObjectId {
    Folder(Uuid),
    Device(Uuid),
    Credential(Uuid),
}

/// Who asks: all SIDs that identify them (their own and their groups').
#[derive(Debug, Clone, Default)]
pub struct Subject {
    pub sids: HashSet<String>,
    pub admin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub object: ObjectId,
    pub principal: String,
    pub role: Role,
    /// Unix time (seconds) from which a just-in-time grant no longer counts;
    /// `None` for a permanent grant.
    pub until: Option<i64>,
}

/// The folder tree and all grants, loaded once per request.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    folder_parent: HashMap<Uuid, Option<Uuid>>,
    device_folder: HashMap<Uuid, Uuid>,
    credential_folder: HashMap<Uuid, Uuid>,
    grants: HashMap<ObjectId, Vec<(String, Role)>>,
}

impl Catalog {
    /// The catalog as of `now` (Unix time, seconds): grants that ran out
    /// before it grant nothing.
    pub fn new(
        folders: impl IntoIterator<Item = (Uuid, Option<Uuid>)>,
        devices: impl IntoIterator<Item = (Uuid, Uuid)>,
        credentials: impl IntoIterator<Item = (Uuid, Uuid)>,
        grants: impl IntoIterator<Item = Grant>,
        now: i64,
    ) -> Self {
        let mut by_object: HashMap<ObjectId, Vec<(String, Role)>> = HashMap::new();
        for grant in grants {
            if grant.until.is_some_and(|until| until <= now) {
                continue;
            }
            by_object
                .entry(grant.object)
                .or_default()
                .push((grant.principal, grant.role));
        }
        Catalog {
            folder_parent: folders.into_iter().collect(),
            device_folder: devices.into_iter().collect(),
            credential_folder: credentials.into_iter().collect(),
            grants: by_object,
        }
    }

    pub fn contains(&self, object: ObjectId) -> bool {
        match object {
            ObjectId::Folder(id) => self.folder_parent.contains_key(&id),
            ObjectId::Device(id) => self.device_folder.contains_key(&id),
            ObjectId::Credential(id) => self.credential_folder.contains_key(&id),
        }
    }

    /// The folder that contains an object (for a folder: its parent).
    pub fn parent(&self, object: ObjectId) -> Option<Uuid> {
        match object {
            ObjectId::Folder(id) => self.folder_parent.get(&id).copied().flatten(),
            ObjectId::Device(id) => self.device_folder.get(&id).copied(),
            ObjectId::Credential(id) => self.credential_folder.get(&id).copied(),
        }
    }

    /// The object and every folder above it, nearest first. Stops at a loop
    /// (which the database prevents) instead of running forever.
    fn path(&self, object: ObjectId) -> Vec<ObjectId> {
        let mut path = vec![object];
        let mut seen = HashSet::new();
        let mut next = self.parent(object);
        while let Some(folder) = next {
            if !seen.insert(folder) {
                break;
            }
            path.push(ObjectId::Folder(folder));
            next = self.folder_parent.get(&folder).copied().flatten();
        }
        path
    }

    /// Whether `folder` is `ancestor` or lies below it.
    pub fn is_within(&self, folder: Uuid, ancestor: Uuid) -> bool {
        self.path(ObjectId::Folder(folder))
            .contains(&ObjectId::Folder(ancestor))
    }

    /// The highest role the subject holds on the object; `None` for none or
    /// for an object that does not exist.
    pub fn effective_role(&self, subject: &Subject, object: ObjectId) -> Option<Role> {
        if !self.contains(object) {
            return None;
        }
        if subject.admin {
            return Some(Role::Manage);
        }
        self.path(object)
            .iter()
            .filter_map(|o| self.grants.get(o))
            .flatten()
            .filter(|(principal, _)| subject.sids.contains(principal))
            .map(|(_, role)| *role)
            .max()
    }

    /// The grants on the object and the folders above it, nearest first:
    /// where each lies, whom it names and which role it gives (#110). It
    /// explains a role; `effective_role` is still what decides.
    pub fn grants_along(&self, object: ObjectId) -> Vec<(ObjectId, &str, Role)> {
        self.path(object)
            .into_iter()
            .flat_map(|on| {
                self.grants
                    .get(&on)
                    .into_iter()
                    .flatten()
                    .map(move |(principal, role)| (on, principal.as_str(), *role))
            })
            .collect()
    }

    /// The grants that give the subject a role on the object, as
    /// `grants_along` lists them: those naming one of its SIDs.
    pub fn reasons(&self, subject: &Subject, object: ObjectId) -> Vec<(ObjectId, &str, Role)> {
        self.grants_along(object)
            .into_iter()
            .filter(|(_, principal, _)| subject.sids.contains(*principal))
            .collect()
    }

    /// Whether the subject holds at least `needed` on the object.
    pub fn authorize(&self, subject: &Subject, needed: Role, object: ObjectId) -> bool {
        self.effective_role(subject, object)
            .is_some_and(|role| role >= needed)
    }

    /// Whether the subject may ask for `role` on the object for a while: they
    /// must see it, and hold less than that. Only `connect` and `reveal` are
    /// given just in time; changing things stays with permanent grants.
    pub fn may_request(&self, subject: &Subject, role: Role, object: ObjectId) -> bool {
        matches!(role, Role::Connect | Role::Reveal)
            && self
                .effective_role(subject, object)
                .is_some_and(|held| held < role)
    }

    /// Whether the subject may approve someone else's request for the
    /// object: the same as managing it. That it is someone else is for the
    /// caller to check, by the requester's identity.
    pub fn may_approve(&self, subject: &Subject, object: ObjectId) -> bool {
        self.authorize(subject, Role::Manage, object)
    }

    /// Creating at the top level (outside every folder) is for administrators.
    pub fn may_create_top_level(&self, subject: &Subject) -> bool {
        subject.admin
    }

    /// Everything the subject may see, with their role; plus the folders
    /// they only see because something visible lies inside (path only).
    pub fn visible(&self, subject: &Subject) -> Visibility {
        let objects = self
            .folder_parent
            .keys()
            .map(|id| ObjectId::Folder(*id))
            .chain(self.device_folder.keys().map(|id| ObjectId::Device(*id)))
            .chain(
                self.credential_folder
                    .keys()
                    .map(|id| ObjectId::Credential(*id)),
            );
        let roles: HashMap<ObjectId, Role> = objects
            .filter_map(|o| Some((o, self.effective_role(subject, o)?)))
            .collect();
        let mut path_only = HashSet::new();
        for object in roles.keys() {
            for ancestor in self.path(*object).into_iter().skip(1) {
                if let ObjectId::Folder(id) = ancestor
                    && !roles.contains_key(&ancestor)
                {
                    path_only.insert(id);
                }
            }
        }
        Visibility { roles, path_only }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Visibility {
    pub roles: HashMap<ObjectId, Role>,
    /// Folders shown only as the way to something visible inside them.
    pub path_only: HashSet<Uuid>,
}

#[cfg(test)]
mod tests;
