use std::{borrow::Borrow, fmt, str::FromStr};

use chrono::{DateTime, FixedOffset};

use crate::{repository::Entity, Url};
mod repository;
pub use repository::UpdateRepo;

#[derive(Debug, PartialEq, Eq)]
pub struct Update {
    update_ref: UpdateRef,
    change: String,
}

impl Update {
    pub(crate) fn new(url: Url, timestamp: DateTime<FixedOffset>, change: String) -> Self {
        Self {
            update_ref: UpdateRef { url, timestamp },
            change,
        }
    }

    pub fn url(&self) -> &Url {
        &self.update_ref.url
    }

    pub fn timestamp(&self) -> &DateTime<FixedOffset> {
        &self.update_ref.timestamp
    }

    pub fn change(&self) -> &str {
        &self.change
    }

    pub fn update_ref(&self) -> &UpdateRef {
        &self.update_ref
    }
}

impl Entity for Update {
    type WriteEvent = UpdateEvent;
}

impl AsRef<UpdateRef> for Update {
    fn as_ref(&self) -> &UpdateRef {
        &self.update_ref
    }
}

impl fmt::Display for Update {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::write(
            f,
            format_args!("Update at {} on {}", self.timestamp().to_rfc3339(), self.url().as_str()),
        )
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub struct UpdateRef {
    pub url: Url,
    pub timestamp: DateTime<FixedOffset>,
}

impl fmt::Display for UpdateRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::write(f, format_args!("{}#{}", self.url.as_str(), self.timestamp.to_rfc3339()))
    }
}

impl FromStr for UpdateRef {
    type Err = UpdateRefParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut url: url::Url = s.parse()?;
        let timestamp = if let Some(fragment) = url.fragment() {
            fragment.parse()?
        } else {
            return Err(UpdateRefParseError::FragmentNotProvided(url));
        };
        url.set_fragment(None);
        Ok(UpdateRef {
            url: url.into(),
            timestamp,
        })
    }
}

impl From<(Url, DateTime<FixedOffset>)> for UpdateRef {
    fn from((url, timestamp): (Url, DateTime<FixedOffset>)) -> Self {
        Self { url, timestamp }
    }
}

#[derive(Debug)]
pub enum UpdateRefParseError {
    ChronoParseError(chrono::ParseError),
    UrlParseError(url::ParseError),
    FragmentNotProvided(url::Url),
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

impl std::error::Error for UpdateRefParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            UpdateRefParseError::ChronoParseError(err) => Some(err),
            UpdateRefParseError::UrlParseError(err) => Some(err),
            UpdateRefParseError::FragmentNotProvided(_) => None,
        }
    }
}

impl fmt::Display for UpdateRefParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateRefParseError::ChronoParseError(err) => write!(f, "Error parsing timestamp : {}", err),
            UpdateRefParseError::UrlParseError(err) => write!(f, "Error parsing url : {}", err),
            UpdateRefParseError::FragmentNotProvided(url) => write!(f, "Timestamp fragment not provided in {}", url),
        }
    }
}

pub struct UpdateRefByUrl<U>(pub U);

impl<U: Borrow<UpdateRef>> Eq for UpdateRefByUrl<U> {}

impl<L: Borrow<UpdateRef>, R: Borrow<UpdateRef>> PartialEq<UpdateRefByUrl<R>> for UpdateRefByUrl<L> {
    fn eq(&self, other: &UpdateRefByUrl<R>) -> bool {
        self.0.borrow() == other.0.borrow()
    }
}

impl<U: Borrow<UpdateRef>> Ord for UpdateRefByUrl<U> {
    fn cmp(&self, UpdateRefByUrl(other): &Self) -> std::cmp::Ordering {
        let UpdateRef { url, timestamp } = self.0.borrow();
        url.cmp(&other.borrow().url)
            .then_with(|| timestamp.cmp(&other.borrow().timestamp))
    }
}

impl<L: Borrow<UpdateRef>, R: Borrow<UpdateRef>> PartialOrd<UpdateRefByUrl<R>> for UpdateRefByUrl<L> {
    fn partial_cmp(&self, UpdateRefByUrl(other): &UpdateRefByUrl<R>) -> Option<std::cmp::Ordering> {
        let UpdateRef { url, timestamp } = self.0.borrow();
        Some(
            url.cmp(&other.borrow().url)
                .then_with(|| timestamp.cmp(&other.borrow().timestamp)),
        )
    }
}

impl From<UpdateRef> for UpdateRefByUrl<UpdateRef> {
    fn from(u: UpdateRef) -> Self {
        UpdateRefByUrl(u)
    }
}

impl From<UpdateRefByUrl<UpdateRef>> for UpdateRef {
    fn from(UpdateRefByUrl(u): UpdateRefByUrl<UpdateRef>) -> Self {
        u
    }
}

#[derive(PartialEq, Eq)]
pub struct UpdateRefByTimestamp(pub UpdateRef);

impl Ord for UpdateRefByTimestamp {
    fn cmp(&self, UpdateRefByTimestamp(other): &Self) -> std::cmp::Ordering {
        let UpdateRefByTimestamp(UpdateRef { url, timestamp }) = self;
        timestamp.cmp(&other.timestamp).then_with(|| url.cmp(&other.url))
    }
}

impl PartialOrd for UpdateRefByTimestamp {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl From<UpdateRef> for UpdateRefByTimestamp {
    fn from(u: UpdateRef) -> Self {
        UpdateRefByTimestamp(u)
    }
}

impl From<UpdateRefByTimestamp> for UpdateRef {
    fn from(UpdateRefByTimestamp(u): UpdateRefByTimestamp) -> Self {
        u
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateEvent {
    /// Any update is added
    Added { url: Url, timestamp: DateTime<FixedOffset> },
    /// A new newest update for a document is added
    New { url: Url, timestamp: DateTime<FixedOffset> },
}

impl UpdateEvent {
    pub(crate) fn added(update: &Update) -> UpdateEvent {
        Self::Added {
            url: update.url().clone(),
            timestamp: *update.timestamp(),
        }
    }

    pub(crate) fn new(update: &Update) -> UpdateEvent {
        Self::New {
            url: update.url().clone(),
            timestamp: *update.timestamp(),
        }
    }
}
