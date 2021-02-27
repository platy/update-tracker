use std::fmt;

use chrono::{DateTime, Utc};
use url::Url;

mod repository;
pub use repository::UpdateRepo;

#[derive(Debug, PartialEq, Eq)]
pub struct Update {
    url: Url,
    timestamp: DateTime<Utc>,
    change: String,
}

impl Update {
    pub fn change(&self) -> &str {
        &self.change
    }
}

impl fmt::Display for Update {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::write(f, format_args!("Update at {} on {}", self.timestamp.to_rfc3339(), self.url.as_str()))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateEvent {
    /// Any update is added
    Added { url: Url, timestamp: DateTime<Utc> },
    /// A new newest update for a document is added
    New { url: Url, timestamp: DateTime<Utc> },
}
