use std::{collections::{BTreeMap, BTreeSet}, convert::TryInto, env::{args}, io::{BufReader, Read}};
use anyhow::*;

use chrono::{DateTime, Utc};
use update_tracker::{doc::{DocRepo, DocumentVersion, iter_history}, tag::{Tag, TagRepo}, update::UpdateRef};

fn main() -> Result<()> {
    let selected_tag = Tag::new(args().nth(1).unwrap());
    println!("Searching tag {}", &selected_tag);

    let repo = TagRepo::new("gitgov-import/out/tag")?;
    let mut tag_index: BTreeMap<Tag, BTreeSet<UpdateRef>> = BTreeMap::new();
    for tag in repo.list_tags()? {
        let updates = tag_index.entry(tag.clone()).or_insert(BTreeSet::new());
        for update in repo.list_updates_in_tag(&tag)? {
            let update = update?;
            updates.insert(update);
        }
    }
    println!("{} tags in tag index with {} taggings", tag_index.len(), tag_index.values().map(|updates| updates.len()).sum::<usize>());
    // for (tag, url, ts) in &tag_index {
    //     println!("{}: {} {}", tag, url, ts);
    // }

    println!("Parsing updates from all documents for update index");
    let repo = DocRepo::new("gitgov-import/out/doc")?;
    let mut update_index = UpdateIndex::new();
    let mut doc_count = 0;
    for doc in repo.list_all(&"https://www.gov.uk/".try_into()?)? {
        let doc = doc?;
        if doc.url().path().ends_with(".html") {
            let mut buf = String::new();
            assert!(BufReader::new(repo.open(&doc).with_context(|| format!("opening contents of {}", &doc))?).read_to_string(&mut buf).with_context(|| format!("reading contents of {}", &doc))? > 0);
            let html = scraper::Html::parse_fragment(&buf);
            for (ts, comment) in iter_history(&html) {
                update_index.insert(&doc, ts, comment)
                    
            }
        }
        doc_count += 1;
    }

    println!("{} updates in {} docs in update index", update_index.len(), doc_count);

    println!("updates tagged {:?}", &selected_tag);
    for update in &tag_index[&selected_tag] {
        println!("{}: {}", &update.timestamp, &update.url);
        if let Some(comment) = update_index.get(update) {
            println!("\t{}", comment);
        }
    }
    Ok(())
}

struct UpdateIndex(BTreeMap<UpdateRef, String>);

impl UpdateIndex {
    fn new() -> UpdateIndex {
        UpdateIndex(BTreeMap::new())
    }

    fn insert(&mut self, doc: &DocumentVersion, ts: DateTime<Utc>, comment: String) {
        self.0.entry(UpdateRef::from((doc.url().clone(), ts)))
            .and_modify(|c| assert!(c == &comment, "{:?} != {:?}", c, &comment))
            .or_insert(comment);
    }

    fn get(&self, update: &UpdateRef) -> Option<&str> {
        println!("{}: {}", &update.timestamp, &update.url);
        let before_update: UpdateRef = (update.url.clone(), update.timestamp - chrono::Duration::days(1)).into();
        if let Some((indexed_update, comment)) = self.0.range(before_update.clone()..).next() {
            assert!(indexed_update == update, "expected {} instead of {}", &update, &indexed_update);
            Some(comment)
        } else {
            None
        }
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}
