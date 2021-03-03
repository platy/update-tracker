use std::{fmt, str::FromStr};

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
        fmt::write(
            f,
            format_args!("Update at {} on {}", self.timestamp.to_rfc3339(), self.url.as_str()),
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct UpdateRef {
    url: Url,
    timestamp: DateTime<Utc>,
}

impl fmt::Display for UpdateRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::write(f, format_args!("{}#{}", self.url.as_str(), self.timestamp.to_rfc3339()))
    }
}

impl FromStr for UpdateRef {
    type Err = UpdateRefParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url: Url = s.parse()?;
        let timestamp = url
            .fragment()
            .ok_or(UpdateRefParseError::FragmentNotProvided)?
            .parse()?;
        Ok(UpdateRef { url, timestamp })
    }
}

impl From<(Url, DateTime<Utc>)> for UpdateRef {
    fn from((url, timestamp): (Url, DateTime<Utc>)) -> Self {
        Self {
            url,
            timestamp,
        }
    }
}

pub enum UpdateRefParseError {
    ChronoParseError(chrono::ParseError),
    UrlParseError(url::ParseError),
    FragmentNotProvided,
}

impl From<chrono::ParseError> for UpdateRefParseError {
    fn from(error: chrono::ParseError) -> Self {
        Self::ChronoParseError(error)
    }
}

impl From<url::ParseError> for UpdateRefParseError {
    fn from(error: url::ParseError) -> Self {
        Self::UrlParseError(error)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateEvent {
    /// Any update is added
    Added { url: Url, timestamp: DateTime<Utc> },
    /// A new newest update for a document is added
    New { url: Url, timestamp: DateTime<Utc> },
}
