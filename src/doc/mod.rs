use std::fmt;

use crate::{repository::Entity, Url};
use chrono::{DateTime, FixedOffset};
use lazy_static::lazy_static;

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

lazy_static! {
    static ref UPDATE_SELECTOR: scraper::Selector =
        scraper::Selector::parse(".app-c-published-dates--history li time").unwrap();
}

/// Iterator over the history of updates in the document
/// Panics if it doesn't recognise the format
pub fn iter_history(doc: &scraper::Html) -> impl Iterator<Item = (DateTime<FixedOffset>, String)> + '_ {
    doc.select(&UPDATE_SELECTOR).map(|time_elem| {
        let time =
            DateTime::parse_from_rfc3339(time_elem.value().attr("datetime").expect("no datetime attribute")).unwrap();
        let sibling = time_elem // faffing around - this is bullshit
            .next_sibling()
            .expect("expected sibling of time element in history");
        let comment_node = sibling.next_sibling().unwrap_or(sibling);
        let comment = if let Some(comment_node) = comment_node.value().as_text() {
            comment_node.trim().to_string()
        } else {
            comment_node
                .children()
                .next()
                .unwrap()
                .value()
                .as_text()
                .unwrap()
                .trim()
                .to_string()
        };
        (time, comment)
    })
}
