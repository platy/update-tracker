use std::fmt;

mod repository;
pub use repository::TagRepo;

use crate::update::UpdateRef;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct Tag {
    name: String,
}

impl Tag {
    pub fn new(name: String) -> Self {
        Self {
            name,
        }
    }
    pub fn name(&self) -> &str {
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
