use std::{
    io::{self, Read},
    ops::Deref,
};

use chrono::{DateTime, FixedOffset};
use htmldiff::htmldiff;
use update_repo::{
    doc::{DocRepo, DocumentVersion},
    update::{Update, UpdateRepo},
    Url,
};

pub(crate) struct Data {
    update_repo: UpdateRepo,
    doc_repo: DocRepo,
    updates: Vec<Update>,
}

impl Data {
    pub fn new() -> Self {
        let update_repo = UpdateRepo::new("../repo/url").unwrap();
        let doc_repo = DocRepo::new("../repo/url").unwrap();

        let mut updates: Vec<_> = update_repo
            .list_all(&"https://www.gov.uk/".parse().unwrap())
            .unwrap()
            .collect::<io::Result<_>>()
            .unwrap();
        updates.sort_by_key(|u| u.timestamp().to_owned());

        Self {
            update_repo,
            doc_repo,
            updates,
        }
    }

    pub fn list_updates(&self) -> &[Update] {
        &self.updates[..]
    }

    pub fn get_update(&self, url: &Url, timestamp: DateTime<FixedOffset>) -> io::Result<Update> {
        self.update_repo.get_update(url.clone(), timestamp)
    }

    pub(crate) fn get_doc_version(&self, url: &Url, timestamp: DateTime<FixedOffset>) -> io::Result<DocumentVersion> {
        self.doc_repo.ensure_version(url.to_owned(), timestamp)
    }

    pub fn iter_doc_versions(&self, url: &Url) -> Option<impl Iterator<Item = DocumentVersion>> {
        self.doc_repo
            .list_versions(url.clone())
            .ok()
            .map(|iter| iter.filter_map(Result::ok))
    }

    pub fn read_doc_to_string(&self, doc: &DocumentVersion) -> DocBody {
        let mut body = String::new();
        self.doc_repo.open(doc).unwrap().read_to_string(&mut body).unwrap();
        DocBody(body)
    }
}

pub struct DocBody(String);

impl DocBody {
    pub fn diff(&self, other: &Self) -> String {
        htmldiff(&self.0, &other.0)
    }

    pub fn with_base_url(self, base_url: &str) -> Self {
        let replace = format!("href=\"{}/", base_url);
        DocBody(self.0.replace("href=\"/", &replace))
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl Deref for DocBody {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
