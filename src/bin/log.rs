use std::{
    collections::{BTreeMap, BTreeSet},
    convert::TryInto,
    error::Error,
    io::{BufReader, Read},
};

use update_tracker::{
    doc::{iter_history, DocRepo},
    tag::{Tag, TagRepo},
    update::UpdateRef,
};

fn main() -> Result<(), Box<dyn Error>> {
    let repo = TagRepo::new("gitgov-import/out/tag")?;
    let mut tag_index: BTreeSet<(Tag, UpdateRef)> = BTreeSet::new();
    for tag in repo.list_tags()? {
        for update in repo.list_updates_in_tag(&tag)? {
            let update = update?;
            tag_index.insert((tag.clone(), (update.url, update.timestamp).into()));
        }
    }
    println!("{} entries in tag index", tag_index.len());
    // for (tag, url, ts) in &tag_index {
    //     println!("{}: {} {}", tag, url, ts);
    // }

    let repo = DocRepo::new("gitgov-import/out/doc", "https://www.gov.uk/".try_into()?)?;
    let mut update_index = BTreeMap::new();
    let mut doc_count = 0;
    for doc in repo.list_all()? {
        let doc = doc?;
        let mut buf = String::new();
        assert!(BufReader::new(repo.open(&doc)?).read_to_string(&mut buf)? > 0);
        let html = scraper::Html::parse_fragment(&buf);
        for (ts, comment) in iter_history(&html) {
            update_index
                .entry(UpdateRef::from((doc.url().clone(), ts)))
                .and_modify(|c| assert!(c == &comment, "{:?} != {:?}", c, &comment))
                .or_insert(comment);
        }
        doc_count += 1;
    }

    println!("{} updates in {} docs in update index", update_index.len(), doc_count);
    Ok(())
}
