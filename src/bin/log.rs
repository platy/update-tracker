use anyhow::*;
use clap::clap_app;
use std::{collections::BTreeSet, convert::TryFrom};

use update_tracker::{
    tag::{Tag, TagRepo},
    update::{UpdateRef, UpdateRepo},
};

fn main() -> Result<()> {
    let matches = clap_app!(myapp =>
        (version: "0.1")
        (author: "Mike Bush <platy@njk.onl>")
        (about: "Lists updates in the update tracker repo")
        // (@arg CONFIG: -c --config +takes_value "Sets a custom config file")
        (@arg FILTER: ... "Filter terms which reduce the output")
        // (@arg verbose: -v --verbose "Print test information verbosely")
    )
    .get_matches();

    let mut filter = Filter::try_from(matches.values_of("FILTER"))?;
    eprintln!("Searching {:?}", &filter);

    let tag_repo = TagRepo::new("gitgov-import/out/tag")?;
    let update_repo = UpdateRepo::new("gitgov-import/out/update")?;

    if let Some(tag) = filter.tags.pop() {
        let mut updates: BTreeSet<UpdateRef> = tag_repo.list_updates_in_tag(&tag)?.collect::<Result<_, _>>()?;
        while let Some(tag) = filter.tags.pop() {
            let mut tmp_updates: BTreeSet<_> = Default::default();
            for update in tag_repo.list_updates_in_tag(&tag)? {
                if let Some(update) = updates.take(&update?) {
                    tmp_updates.insert(update);
                }
            }
            updates = tmp_updates;
        }
        for update in updates {
            println!("{}: {}", &update.timestamp, &update.url);
            let comment = update_repo.get_update(update.url.clone(), update.timestamp)?;
            println!("\t{}", comment.change());
        }
    } else {
        todo!("Needs list all updates in repo");
    }
    Ok(())
}

#[derive(Debug)]
struct Filter {
    tags: Vec<Tag>,
}

impl<'s> TryFrom<Option<clap::Values<'s>>> for Filter {
    type Error = anyhow::Error;

    fn try_from(values: Option<clap::Values<'s>>) -> Result<Self, Self::Error> {
        let mut tags = vec![];
        if let Some(values) = values {
            for mut token in values {
                if token.starts_with("#\"") {
                    // tag until next double quote
                    token = &token[2..(2 + token[2..].find('"').context("Missing matching double quote")?)];
                } else if token.starts_with('#') {
                    // tag until next whitespace
                    token = &token[1..];
                }
                tags.push(Tag::new(token.to_owned()));
            }
        }
        Ok(Filter { tags })
    }
}
