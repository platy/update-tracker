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

#[derive(Debug, PartialEq, Eq)]
pub enum DocEvent {
    Created { url: Url },
    Updated { url: Url, timestamp: DateTime<Utc> },
}
