use std::{collections::{BTreeMap, BTreeSet}, convert::TryInto, env::{args}, error::Error, io::{BufReader, Read}};

use update_tracker::{
    doc::{iter_history, DocRepo},
    tag::{Tag, TagRepo},
    update::UpdateRef,
};

fn main() -> Result<(), Box<dyn Error>> {
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

    let repo = DocRepo::new("gitgov-import/out/doc")?;
    let mut update_index = BTreeMap::new();
    let mut doc_count = 0;
    for doc in repo.list_all(&"https://www.gov.uk/".try_into()?)? {
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

    println!("updates tagged {:?}", &selected_tag);
    for update in &tag_index[&selected_tag] {
        println!("{}: {}", &update.timestamp, &update.url);
        if let Some(comment) = update_index.get(update) {
            println!("\t{}", comment);
        } else {
            let before_update: UpdateRef = (update.url.clone(), update.timestamp - chrono::Duration::days(1)).into();
            println!("\t{:?}", update_index.range(before_update..).take(1).collect::<Vec<_>>())
        }
    }
    Ok(())
}
