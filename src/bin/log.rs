use anyhow::*;
use chrono::{Datelike, NaiveDate, NaiveDateTime};
use clap::clap_app;
use std::{
    collections::BTreeSet,
    convert::TryFrom,
    ops::{Bound, RangeBounds},
};
use url::Url;

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
        let mut updates: BTreeSet<UpdateRef> = tag_repo
            .list_updates_in_tag(&tag)?
            .filter(|update_ref| {
                update_ref
                    .as_ref()
                    .map_or(true, |update_ref| filter.filter_update_ref(update_ref))
            })
            .collect::<Result<_, _>>()?;
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
    /// Filter to only updates with the intersection of these tags
    tags: Vec<Tag>,
    /// Filter to only updates on urls starting with this url prefix
    url_prefix: Option<Url>,
    /// Filter to only updates published within a date range
    date_range: (Bound<NaiveDateTime>, Bound<NaiveDateTime>),
}

impl<'s> TryFrom<Option<clap::Values<'s>>> for Filter {
    type Error = anyhow::Error;

    fn try_from(values: Option<clap::Values<'s>>) -> Result<Self, Self::Error> {
        let mut tags = vec![];
        let mut url_prefix = None;
        let mut date_range = (Bound::Unbounded, Bound::Unbounded);
        if let Some(values) = values {
            for token in values {
                if let Some(mut tag) = token.strip_prefix("#\"") {
                    // tag until next double quote
                    tag = &tag[..(2 + tag
                        .find('"')
                        .context(format!("Missing matching double quote on '{}'", tag))?)];
                    tags.push(Tag::new(tag.to_owned()));
                } else if let Some(tag) = token.strip_prefix('#') {
                    // tag until next whitespace
                    tags.push(Tag::new(tag.to_owned()));
                } else if token.starts_with("https://www.gov.uk/") {
                    url_prefix = Some(token.parse()?);
                } else if let Some((from, to)) = token.split_once("..") {
                    date_range = (
                        Filter::parse_date_bound(from)?.map_or(Bound::Unbounded, Bound::Included),
                        Filter::parse_date_bound(to)?.map_or(Bound::Unbounded, Bound::Excluded),
                    );
                } else {
                    bail!("Unrecognised filter {}", token);
                }
            }
        }
        Ok(Filter {
            tags,
            url_prefix,
            date_range,
        })
    }
}

impl Filter {
    fn filter_update_ref(&self, update_ref: &UpdateRef) -> bool {
        if let Some(url_prefix) = &self.url_prefix {
            if !update_ref.to_string().starts_with(&url_prefix.to_string()) {
                return false;
            }
        }
        self.date_range.contains(&update_ref.timestamp.naive_local())
    }

    fn parse_date_bound(s: &str) -> Result<Option<NaiveDateTime>> {
        if s.is_empty() {
            return Ok(None);
        }
        let mut date = NaiveDate::from_ymd(0, 1, 1);
        let mut date_parts = s.split('-');
        date = date
            .with_year(date_parts.next().unwrap_or("").parse().context("Error parsing year")?)
            .context("Invalid year")?;
        if let Some(m) = date_parts.next().map(str::parse).transpose()? {
            date = date.with_month(m).context("Error parsing month")?;
        }
        if let Some(d) = date_parts.next().map(str::parse).transpose()? {
            date = date.with_day(d).context("Error parsing day")?;
        }
        Ok(Some(date.and_hms(0, 0, 0)))
    }
}
