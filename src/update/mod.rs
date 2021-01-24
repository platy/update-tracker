use chrono::{DateTime, Utc};
use url::Url;

struct Update {
    url: Url,
    timestamp: DateTime<Utc>,
    change: String,
}
