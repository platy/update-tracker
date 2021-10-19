use std::{
    cmp::Reverse,
    collections::BTreeMap,
    io::{self, Read},
    ops::Deref,
};

use chrono::{DateTime, FixedOffset};
use htmldiff::htmldiff;
use qp_trie::Trie;
use update_repo::{
    doc::{DocRepo, DocumentVersion},
    tag::{Tag, TagRepo},
    update::{Update, UpdateRef, UpdateRepo},
    Url,
};

pub(crate) struct Data {
    update_repo: UpdateRepo,
    doc_repo: DocRepo,
    /// All updates in ascending timestamp order
    updates: Vec<&'static Update>,
    /// all updates in url and then timestamp order with tags
    index: Trie<Url, BTreeMap<DateTime<FixedOffset>, (&'static Update, Vec<&'static Tag>)>>,
    all_tags: Vec<String>,
}

impl Data {
    pub fn new() -> Self {
        let update_repo = UpdateRepo::new("../repo/url").unwrap();
        let doc_repo = DocRepo::new("../repo/url").unwrap();

        let mut updates: Vec<_> = vec![];
        let mut tags: Trie<_, BTreeMap<_, _>> = Trie::new();
        for update in update_repo.list_all(&"https://www.gov.uk/".parse().unwrap()).unwrap() {
            let r = &*Box::leak(Box::new(update.unwrap()));
            updates.push(r);
            tags.entry(r.url().clone())
                .or_insert_with(Default::default)
                .insert(*r.timestamp(), (r, Vec::with_capacity(2)));
        }
        updates.sort_by_key(|u| u.timestamp().to_owned());

        let tag_repo = TagRepo::new("../repo/tag").unwrap();
        let mut all_tags = vec![];
        for tag in tag_repo.list_tags().unwrap() {
            println!("Tag {}", tag.name());
            all_tags.push(tag.name().to_owned());
            let tag = &*Box::leak(Box::new(tag));
            for ur in tag_repo.list_updates_in_tag(tag).unwrap() {
                let ur = ur.unwrap();
                let (_update, tags) = tags
                    .get_mut(&ur.url)
                    .expect("no tag entry for url")
                    .get_mut(&ur.timestamp)
                    .expect("no tag entry for timestamp");
                tags.push(tag);
            }
        }

        Self {
            update_repo,
            doc_repo,
            updates,
            index: tags,
            all_tags,
        }
    }

    pub fn list_updates(&self, base: &Url) -> Box<dyn Iterator<Item = &'static Update> + '_> {
        if base.as_str() == "https://www.gov.uk" {
            let iter = self.updates.iter().copied().rev();
            Box::new(iter)
        } else {
            let mut filtered: Vec<_> = self
                .index
                .iter_prefix(base)
                .flat_map(|(_, map)| map.iter().map(|(_, (update, _))| update))
                .copied()
                .collect();
            filtered.sort_by_key(|update| Reverse(update.timestamp()));
            Box::new(filtered.into_iter())
        }
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
        self.index
            .get(&ur.url)
            .unwrap()
            .get(&ur.timestamp)
            .unwrap()
            .1
            .as_slice()
    }

    pub fn all_tags(&self) -> impl Iterator<Item = &String> {
        self.all_tags.iter()
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
