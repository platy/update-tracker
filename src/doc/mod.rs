use std::fmt;

use crate::{repository::Entity, Url};
use chrono::{DateTime, FixedOffset};

mod repository;
pub use repository::DocRepo;

#[derive(Debug, PartialEq, Eq)]
pub struct Document {
    url: Url,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DocumentVersion {
    url: Url,
    timestamp: DateTime<FixedOffset>,
}

impl DocumentVersion {
    pub fn url(&self) -> &Url {
        &self.url
    }
}

impl Entity for DocumentVersion {
    type WriteEvent = DocEvent;
}

impl fmt::Display for DocumentVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::write(
            f,
            format_args!(
                "Doc retrieved at {} on {}",
                self.timestamp.to_rfc3339(),
                self.url.as_str()
            ),
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DocEvent {
    Created { url: Url },
    Updated { url: Url, timestamp: DateTime<FixedOffset> },
    Deleted { url: Url, timestamp: DateTime<FixedOffset> },
}

impl DocEvent {
    pub(crate) fn created(doc: &DocumentVersion) -> Self {
        Self::Created { url: doc.url.clone() }
    }

    pub(crate) fn updated(doc: &DocumentVersion) -> Self {
        Self::Updated {
            url: doc.url.clone(),
            timestamp: doc.timestamp,
        }
    }

    pub(crate) fn deleted(doc: &DocumentVersion) -> Self {
        Self::Deleted {
            url: doc.url.clone(),
            timestamp: doc.timestamp,
        }
    }
}
