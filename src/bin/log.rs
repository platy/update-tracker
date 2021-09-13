use anyhow::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    env::args,
};

use update_tracker::{
    tag::{Tag, TagRepo},
    update::{UpdateRef, UpdateRepo},
};

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
    println!(
        "{} tags in tag index with {} taggings",
        tag_index.len(),
        tag_index.values().map(|updates| updates.len()).sum::<usize>()
    );
    // for (tag, url, ts) in &tag_index {
    //     println!("{}: {} {}", tag, url, ts);
    // }

    let repo = UpdateRepo::new("gitgov-import/out/update")?;

    println!("updates tagged {:?}", &selected_tag);
    for update in &tag_index[&selected_tag] {
        println!("{}: {}", &update.timestamp, &update.url);
        let comment = repo.get_update(update.url.clone(), update.timestamp)?;
        println!("\t{}", comment.change());
    }
    Ok(())
}
