use chrono::{DateTime, Utc};
use url::Url;

mod repository;

#[derive(Debug, PartialEq, Eq)]
struct Document {
    url: Url,
}

#[derive(Debug, PartialEq, Eq)]
struct DocumentVersion {
    url: Url,
    timestamp: DateTime<Utc>,
}

#[derive(Debug, PartialEq, Eq)]
enum DocEvent {
    Created { url: Url },
    Updated { url: Url, timestamp: DateTime<Utc> },
}
