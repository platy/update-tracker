use std::{
    collections::BTreeMap,
    io::{self, Read},
    ops::Deref,
};

use chrono::{DateTime, FixedOffset};
use htmldiff::htmldiff;
use update_repo::{
    doc::{DocRepo, DocumentVersion},
    tag::{Tag, TagRepo},
    update::{Update, UpdateRef, UpdateRefByUrl, UpdateRepo},
    Url,
};

pub(crate) struct Data {
    update_repo: UpdateRepo,
    doc_repo: DocRepo,
    /// All updates in ascending timestamp order
    updates: Vec<&'static Update>,
    /// all updates in url and then timestamp order with tags
    tags: BTreeMap<UpdateRefByUrl<UpdateRef>, (&'static Update, Vec<&'static Tag>)>,
}

impl Data {
    pub fn new() -> Self {
        let update_repo = UpdateRepo::new("../repo/url").unwrap();
        let doc_repo = DocRepo::new("../repo/url").unwrap();

        let mut updates: Vec<_> = vec![];
        let mut tags = BTreeMap::new();
        for update in update_repo.list_all(&"https://www.gov.uk/".parse().unwrap()).unwrap() {
            let r = &*Box::leak(Box::new(update.unwrap()));
            updates.push(r);
            tags.insert(UpdateRefByUrl(r.update_ref().clone()), (r, Vec::with_capacity(2)));
        }
        updates.sort_by_key(|u| u.timestamp().to_owned());

        let tag_repo = TagRepo::new("../repo/tag").unwrap();
        for tag in tag_repo.list_tags().unwrap() {
            println!("Tag {}", tag.name());
            let tag = &*Box::leak(Box::new(tag));
            for ur in tag_repo.list_updates_in_tag(tag).unwrap() {
                let ur = ur.unwrap();
                let (_update, tags) = tags
                    .get_mut(&UpdateRefByUrl(ur.clone()))
                    .expect("no tag entry for ref");
                tags.push(tag);
            }
        }

        Self {
            update_repo,
            doc_repo,
            updates,
            tags,
        }
    }

    pub fn list_updates(&self) -> &[&'static Update] {
        &self.updates
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

    pub fn get_tags(&self, ur: &UpdateRef) -> &[&Tag] {
        self.tags.get(&UpdateRefByUrl(ur.clone())).unwrap().1.as_slice()
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
