use std::fmt;

use chrono::{DateTime, Utc};
use url::Url;

mod repository;
pub use repository::DocRepo;

#[derive(Debug, PartialEq, Eq)]
pub struct Document {
    url: Url,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DocumentVersion {
    url: Url,
    timestamp: DateTime<Utc>,
}

impl fmt::Display for DocumentVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::write(f, format_args!("Doc retrieved at {} on {}", self.timestamp.to_rfc3339(), self.url.as_str()))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DocEvent {
    Created { url: Url },
    Updated { url: Url, timestamp: DateTime<Utc> },
}
