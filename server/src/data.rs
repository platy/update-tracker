use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashSet},
    io::{self, Read},
    ops::Deref,
    path::Path,
    sync::Arc,
};

use chrono::{DateTime, FixedOffset};
use htmldiff::htmldiff;
use qp_trie::Trie;
use simsearch::SimSearch;
use update_repo::{
    doc::{DocRepo, DocumentVersion},
    tag::{Tag, TagRepo},
    update::{Update, UpdateRef, UpdateRepo},
    Url,
};

type TimestampSubIndex = BTreeMap<DateTime<FixedOffset>, (Arc<Update>, HashSet<Arc<Tag>>)>;

pub struct Data {
    doc_repo: DocRepo,
    /// All updates in ascending timestamp order
    updates: Vec<Arc<Update>>,
    /// all updates in url and then timestamp order with tags
    index: Trie<Url, TimestampSubIndex>,
    all_tags: Vec<String>,
    /// full text index of the updat change field, it's keyed on the index on `self.index`
    change_index: SimSearch<UpdateRef>,
}

impl Data {
    pub fn load(repo_base: &Path) -> Self {
        let update_repo = UpdateRepo::new(repo_base.join("url")).unwrap();
        let doc_repo = DocRepo::new(repo_base.join("url")).unwrap();

        let change_index = SimSearch::new();
        let updates: Vec<_> = vec![];
        let index: Trie<_, BTreeMap<_, _>> = Trie::new();

        let tag_repo = TagRepo::new(repo_base.join("tag")).unwrap();
        let all_tags = vec![];

        let mut this = Self {
            doc_repo,
            updates,
            index,
            all_tags,
            change_index,
        };

        for update in update_repo.list_all(&"https://www.gov.uk/".parse().unwrap()).unwrap() {
            let update = update.unwrap();
            this.append_update(update);
        }
        this.updates.sort_by_key(|u| u.timestamp().to_owned());

        for tag in tag_repo.list_tags().unwrap() {
            println!("Tag {}", tag.name());
            this.all_tags.push(tag.name().to_owned());
            let tag = Arc::new(tag);
            for ur in tag_repo.list_updates_in_tag(&tag).unwrap() {
                let ur = ur.unwrap();
                this.add_tag(ur, tag.clone());
            }
        }

        this
    }

    pub fn append_update(&mut self, update: Update) {
        self.change_index.insert(update.update_ref().clone(), update.change());
        let update = Arc::new(update);
        self.updates.push(update.clone());
        self.index
            .entry(update.url().clone())
            .or_insert_with(Default::default)
            .insert(*update.timestamp(), (update, HashSet::with_capacity(2)));
    }

    pub fn add_tag(&mut self, ur: UpdateRef, tag: Arc<Tag>) {
        let (_update, tags) = self
            .index
            .get_mut(&ur.url)
            .expect("no tag entry for url")
            .get_mut(&ur.timestamp)
            .expect("no tag entry for timestamp");
        tags.insert(tag);
    }

    pub fn list_updates(
        &self,
        base: &Url,
        change_terms: Option<String>,
        tag: Option<Tag>,
    ) -> Box<dyn Iterator<Item = &Update> + '_> {
        let change_matches = change_terms.map(|change_terms| {
            self.change_index
                .search(&change_terms)
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
        });

        let match_tag_and_change = move |u: &&Update| {
            if let Some(tag) = &tag {
                if !self.get_tags(u.update_ref()).contains(tag) {
                    return false;
                }
            }
            if let Some(change_matches) = &change_matches {
                if !change_matches.contains(u.update_ref()) {
                    return false;
                }
            }
            true
        };

        if base.as_str() == "https://www.gov.uk" {
            let iter = self.updates.iter().rev().map(Deref::deref);
            Box::new(iter.filter(match_tag_and_change))
        } else {
            let mut filtered: Vec<_> = self
                .index
                .iter_prefix(base)
                .flat_map(|(_, map)| map.iter().map(|(_, (update, _))| update))
                .collect();
            filtered.sort_by_key(|update| Reverse(update.timestamp()));
            Box::new(filtered.into_iter().map(Deref::deref).filter(match_tag_and_change))
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
