use anyhow::*;
use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, NaiveDateTime, Utc};
use clap::clap_app;
use std::{
    collections::BTreeSet,
    convert::TryFrom,
    fmt,
    ops::{Bound, RangeBounds},
};

use update_repo::{
    tag::{Tag, TagRepo},
    update::{Update, UpdateRef, UpdateRefByTimestamp, UpdateRefByUrl, UpdateRepo},
};

fn main() -> Result<()> {
    let matches = clap_app!(myapp =>
        (version: "0.1")
        (author: "Mike Bush <platy@njk.onl>")
        (about: "Lists updates in the update tracker repo")
        (@arg ORDER: -o --order +takes_value "Orders updates, either [u]rl or [t]imestamp (default)")
        (@arg FILTER: ... "Filter terms which reduce the output")
        // (@arg verbose: -v --verbose "Print test information verbosely")
    )
    .get_matches();

    let filter = Filter::try_from(matches.values_of("FILTER"))?;
    eprintln!("Searching {:?}", &filter);

    match matches.value_of("ORDER") {
        Some("u" | "url") => list_updates::<UpdateRefByUrl>(filter)?,
        Some("t" | "time" | "timestamp") | None => list_updates::<UpdateRefByTimestamp>(filter)?,
        Some(other) => bail!("Unknown sort ordering '{}', expected 'url' or 'timestamp'", other),
    }

    Ok(())
}

fn list_updates<O>(mut filter: Filter) -> Result<(), Error>
where
    O: Ord + From<UpdateRef> + Into<UpdateRef>,
{
    let tag_repo = TagRepo::new("repo/tag")?;
    let update_repo = UpdateRepo::new("repo/url")?;
    if let Some(tag) = filter.tags.pop() {
        let mut updates: BTreeSet<O> = tag_repo
            .list_updates_in_tag(&tag)?
            .filter(|update_ref| {
                update_ref
                    .as_ref()
                    .map_or(true, |update_ref| filter.filter_update_ref(update_ref))
            })
            .map(|r| r.map(Into::into))
            .collect::<Result<_, _>>()?;
        while let Some(tag) = filter.tags.pop() {
            let mut tmp_updates: BTreeSet<_> = Default::default();
            for update in tag_repo.list_updates_in_tag(&tag)? {
                if let Some(update) = updates.take(&update?.into()) {
                    tmp_updates.insert(update);
                }
            }
            updates = tmp_updates;
        }
        let updates = updates
            .into_iter()
            .map(Into::into)
            .map(|update_ref| update_repo.get_update(update_ref.url.clone(), update_ref.timestamp));
        print_updates(updates, &update_repo)?;
    } else {
        let updates = update_repo
            .list_all(&"https://www.gov.uk/".parse().unwrap())?
            .filter(|update| {
                update
                    .as_ref()
                    .map_or(true, |update| filter.filter_update_ref(update.as_ref()))
            });
        print_updates(updates, &update_repo)?;
    }
    Ok(())
}

fn print_updates<E>(updates: impl IntoIterator<Item = Result<Update, E>>, update_repo: &UpdateRepo) -> Result<(), Error>
where
    E: fmt::Debug,
{
    for update in updates {
        let update = update.unwrap();
        println!("{}: {}", &update.timestamp(), &update.url());
        let comment = update_repo.get_update(update.url().clone(), *update.timestamp())?;
        println!("\t{}", comment.change());
    }
    Ok(())
}

#[derive(Debug)]
struct Filter {
    /// Filter to only updates with the intersection of these tags
    tags: Vec<Tag>,
    /// Filter to only updates on urls starting with this url prefix
    url_prefix: Option<url::Url>,
    /// Filter to only updates published within a date range
    date_range: (Bound<NaiveDateTime>, Bound<NaiveDateTime>),
    /// Filter by age
    age_range: (Bound<Duration>, Bound<Duration>),
}

impl<'s> TryFrom<Option<clap::Values<'s>>> for Filter {
    type Error = anyhow::Error;

    fn try_from(values: Option<clap::Values<'s>>) -> Result<Self, Self::Error> {
        let mut tags = vec![];
        let mut url_prefix = None;
        let mut date_range = (Bound::Unbounded, Bound::Unbounded);
        let mut age_range = (Bound::Unbounded, Bound::Unbounded);
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
                } else if let Some((from, to)) = token.split_once("...") {
                    age_range = (
                        Filter::parse_age_bound(to)?.map_or(Bound::Unbounded, Bound::Included),
                        Filter::parse_age_bound(from)?.map_or(Bound::Unbounded, Bound::Excluded),
                    );
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
            age_range,
        })
    }
}

impl Filter {
    fn filter_update_ref(&self, update_ref: &UpdateRef) -> bool {
        if let Some(url_prefix) = &self.url_prefix {
            if !update_ref.url.as_str().starts_with(url_prefix.as_str()) {
                return false;
            }
        }
        self.date_range.contains(&update_ref.timestamp.naive_local())
            && self
                .age_range
                .contains(&(DateTime::<FixedOffset>::from(Utc::now()) - update_ref.timestamp))
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

    fn parse_age_bound(mut s: &str) -> Result<Option<Duration>> {
        if s.is_empty() {
            return Ok(None);
        }
        let mut duration = Duration::seconds(0);
        while !s.is_empty() {
            // this panics
            let (multiple, rest) = s.split_at(s.chars().take_while(|&c| c.is_numeric()).count());
            let (unit, rest) = rest.split_at(rest.chars().take_while(|&c| c.is_alphanumeric()).count());
            match unit.to_lowercase().as_str() {
                "y" | "year" | "years" => {
                    duration =
                        duration + Duration::weeks(53 * multiple.parse::<i64>().context("Failed to parse number")?)
                }
                "m" | "month" | "months" => {
                    duration =
                        duration + Duration::days(30 * multiple.parse::<i64>().context("Failed to parse number")?)
                }
                "w" | "week" | "weeks" => {
                    duration = duration + Duration::weeks(multiple.parse::<i64>().context("Failed to parse number")?)
                }
                "d" | "day" | "days" => {
                    duration = duration + Duration::days(multiple.parse::<i64>().context("Failed to parse number")?)
                }
                other => bail!("Unknown age unit {}", other),
            }
            s = rest;
        }
        Ok(Some(duration))
    }
}
