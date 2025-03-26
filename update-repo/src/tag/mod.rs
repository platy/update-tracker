use std::{fmt, ops::Deref};

mod repository;
pub use repository::TagRepo;

use crate::{repository::Entity, update::UpdateRef};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct Tag {
    name: String,
}

impl Tag {
    pub fn new(name: String) -> Self {
        Self { name }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Entity for Tag {
    type WriteEvent = TagEvent;
}

impl Deref for Tag {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.name
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.name.fmt(f)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TagEvent {
    /// An update is tagged
    UpdateTagged { tag: Tag, update_ref: UpdateRef },
    /// A new tag is added
    TagCreated { tag: Tag },
}
impl TagEvent {
    pub(crate) fn tag_created(tag: Tag) -> Self {
        Self::TagCreated { tag }
    }

    pub(crate) fn update_tagged(tag: Tag, update_ref: &UpdateRef) -> Self {
        Self::UpdateTagged {
            tag,
            update_ref: update_ref.clone(),
        }
    }
}
