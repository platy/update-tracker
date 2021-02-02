use chrono::{DateTime, Utc};
use url::Url;

mod repository;

#[derive(Debug, PartialEq, Eq)]
struct Update {
    url: Url,
    timestamp: DateTime<Utc>,
    change: String,
}

#[derive(Debug, PartialEq, Eq)]
enum UpdateEvent {
    /// Any update is added
    Added { url: Url, timestamp: DateTime<Utc> },
    /// A new newest update for a document is added
    New { url: Url, timestamp: DateTime<Utc> },
}
