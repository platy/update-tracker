use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashSet},
    io::{self, Read},
    ops::Deref,
    sync::Arc,
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

type TimestampSubIndex = BTreeMap<DateTime<FixedOffset>, (Arc<Update>, HashSet<Arc<Tag>>)>;

pub(crate) struct Data {
    doc_repo: DocRepo,
    /// All updates in ascending timestamp order
    updates: Vec<Arc<Update>>,
    /// all updates in url and then timestamp order with tags
    index: Trie<Url, TimestampSubIndex>,
    all_tags: Vec<String>,
}

impl Data {
    pub fn new() -> Self {
        let update_repo = UpdateRepo::new("../repo/url").unwrap();
        let doc_repo = DocRepo::new("../repo/url").unwrap();

        let mut updates: Vec<_> = vec![];
        let mut index: Trie<_, BTreeMap<_, _>> = Trie::new();
        for update in update_repo.list_all(&"https://www.gov.uk/".parse().unwrap()).unwrap() {
            let r = Arc::new(update.unwrap());
            updates.push(r.clone());
            index
                .entry(r.url().clone())
                .or_insert_with(Default::default)
                .insert(*r.timestamp(), (r, HashSet::with_capacity(2)));
        }
        updates.sort_by_key(|u| u.timestamp().to_owned());

        let tag_repo = TagRepo::new("../repo/tag").unwrap();
        let mut all_tags = vec![];
        for tag in tag_repo.list_tags().unwrap() {
            println!("Tag {}", tag.name());
            all_tags.push(tag.name().to_owned());
            let tag = Arc::new(tag);
            for ur in tag_repo.list_updates_in_tag(&tag).unwrap() {
                let ur = ur.unwrap();
                let (_update, tags) = index
                    .get_mut(&ur.url)
                    .expect("no tag entry for url")
                    .get_mut(&ur.timestamp)
                    .expect("no tag entry for timestamp");
                tags.insert(tag.clone());
            }
        }

        Self {
            doc_repo,
            updates,
            index,
            all_tags,
        }
    }

    pub fn list_updates(&self, base: &Url) -> Box<dyn Iterator<Item = &Update> + '_> {
        if base.as_str() == "https://www.gov.uk" {
            let iter = self.updates.iter().rev().map(Deref::deref);
            Box::new(iter)
        } else {
            let mut filtered: Vec<_> = self
                .index
                .iter_prefix(base)
                .flat_map(|(_, map)| map.iter().map(|(_, (update, _))| update))
                .collect();
            filtered.sort_by_key(|update| Reverse(update.timestamp()));
            Box::new(filtered.into_iter().map(Deref::deref))
        }
    }

    pub fn get_updates(&self, url: &Url) -> Option<&TimestampSubIndex> {
        self.index.get(url)
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

    pub fn get_tags(&self, ur: &UpdateRef) -> &HashSet<Arc<Tag>> {
        &self.index.get(&ur.url).unwrap().get(&ur.timestamp).unwrap().1
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
