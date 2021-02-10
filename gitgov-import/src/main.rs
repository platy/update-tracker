use std::{fs::remove_dir_all, io, str::FromStr, sync::mpsc};

use anyhow::{bail, format_err, Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use git2::{Commit, Repository};
use update_tracker::update::UpdateRepo;
use url::Url;

fn main() -> Result<()> {
    const REPO_BASE: &str = "./out/update";
    remove_dir_all(REPO_BASE)?;

    let repo = Repository::open(dotenv::var("GITGOV_REPO")?)?;
    let reference = repo.find_reference(&dotenv::var("GITGOV_REF")?)?;
    let mut commit = reference.peel_to_commit()?;

    let (events, _blah) = mpsc::channel();
    let mut update_repo = UpdateRepo::new(REPO_BASE, events)?;

    loop {
        if commit.author().email().unwrap() == "info@gov.uk" {
            let extractor = Extractor {
                commit: &commit,
                repo: &repo,
            };
            if let Err(error) = import_commit(extractor, &mut update_repo) {
                println!("Error on {} : {}", commit.id(), error);
                if !error.to_string().contains("Too many files") {
                    break;
                }
            }
        } else {
            println!("Non-update commit : {}", commit.message().unwrap());
        }

        if let Some(parent) = commit.parents().next() {
            commit = parent;
        } else {
            break;
        }
    }

    Ok(())
}

fn import_commit(extractor: Extractor, repo: &mut UpdateRepo) -> Result<()> {
    // println!("import {}", extractor.commit.message().unwrap());
    let url = extractor.url()?;
    let ts = extractor.timestamp()?;
    let change = extractor.message()?;
    let _tag = extractor.tag()?;
    match repo.create(url.clone(), ts, change) {
        Ok(update) => {
            println!("create {:?}", &update);
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            let existing = repo.get_update(url, ts)?;
            if existing.change() == change {
                println!("exists {:?}", &existing);
                Ok(())
            } else {
                Err(format_err!(
                    "Update exists with different content, expecting `{}`, found `{}`",
                    change,
                    existing.change()
                ))
            }
        }
        Err(err) => Err(err).context("error writing update"),
    }
}

struct Extractor<'r> {
    repo: &'r git2::Repository,
    commit: &'r git2::Commit<'r>,
}

impl<'r> Extractor<'r> {
    fn url(&self) -> Result<Url> {
        let tree = self.commit.tree()?;
        let parent_tree = self.commit.parents().next().as_ref().map(Commit::tree).transpose()?;
        let diff = self.repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        let files: Vec<_> = diff.deltas().collect();
        if files.len() != 1 {
            bail!("Too many files in commit {}", self.commit.id());
        }
        let path = files[0].new_file().path().unwrap();
        // println!("path {:?}", path);
        let url = Url::from_str(&format!("https://www.gov.uk/{}", path.to_str().unwrap()))?;
        // println!("url {}", url);
        Ok(url)
    }

    fn timestamp(&self) -> Result<DateTime<Utc>> {
        let date = self.commit.message().unwrap().split(": ").nth(0).unwrap();
        // println!("date{}", date);
        const DATE_FORMAT: &str = "%I:%M%p, %d %B %Y";
        let local_ts = NaiveDateTime::parse_from_str(date, DATE_FORMAT).context("parsing timestamp")?;
        Ok(DateTime::from_utc(local_ts, Utc)) //FIXME, it's not really UTC
    }

    fn message(&self) -> Result<&str> {
        let message = self.commit.message().unwrap().split(": ").nth(1).unwrap();
        let message = message.split(" [").next().unwrap();
        Ok(message)
    }

    fn tag(&self) -> Result<&str> {
        let tag = self
            .commit
            .message()
            .unwrap()
            .split(" [")
            .nth(1)
            .unwrap()
            .split(']')
            .next()
            .unwrap();
        Ok(tag)
    }
}
