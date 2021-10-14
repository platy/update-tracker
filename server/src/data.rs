use std::{
    cmp::Reverse,
    io::{self, Read},
};

use chrono::{DateTime, FixedOffset};
use update_repo::{
    doc::{DocRepo, DocumentVersion},
    update::{Update, UpdateRepo},
    Url,
};

pub(crate) struct Data {
    update_repo: UpdateRepo,
    doc_repo: DocRepo,
}

impl Data {
    pub fn new() -> Self {
        let update_repo = UpdateRepo::new("../repo/url").unwrap();
        let doc_repo = DocRepo::new("../repo/url").unwrap();
        Self { update_repo, doc_repo }
    }

    pub fn list_updates(&self) -> Vec<Update> {
        let mut updates: Vec<_> = self
            .update_repo
            .list_all(&"https://www.gov.uk/".parse().unwrap())
            .unwrap()
            .collect::<io::Result<_>>()
            .unwrap();
        updates.sort_by_key(|u| Reverse(u.timestamp().to_owned()));
        updates
    }

    pub fn get_update(&self, url: &Url, timestamp: DateTime<FixedOffset>) -> Result<Update, io::Error> {
        self.update_repo.get_update(url.clone(), timestamp)
    }

    pub fn iter_doc_versions(&self, url: &Url) -> Option<impl Iterator<Item = DocumentVersion>> {
        self.doc_repo
            .list_versions(url.clone())
            .ok()
            .map(|iter| iter.filter_map(Result::ok))
    }

    pub fn read_doc_to_string(&self, doc: &DocumentVersion) -> String {
        let mut body = String::new();
        self.doc_repo.open(doc).unwrap().read_to_string(&mut body).unwrap();
        body
    }
}
