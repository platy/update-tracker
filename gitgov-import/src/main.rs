use std::{
    collections::HashSet,
    fs::remove_dir_all,
    io,
    str::{from_utf8, FromStr},
};

use anyhow::{bail, ensure, format_err, Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use git2::{Blob, Commit, Diff, Oid, Repository};
use html5ever::serialize::{HtmlSerializer, Serialize, SerializeOpts, Serializer, TraversalScope};
use io::{Read, Write};
use update_tracker::{doc::DocRepo, update::UpdateRepo};
use url::Url;

fn main() -> Result<()> {
    const UPDATE_REPO_BASE: &str = "./out/update";
    const DOC_REPO_BASE: &str = "./out/doc";
    let _ = remove_dir_all(UPDATE_REPO_BASE);
    let _ = remove_dir_all(DOC_REPO_BASE);

    let repo = Repository::open(dotenv::var("GITGOV_REPO")?)?;
    let reference = repo.find_reference(&dotenv::var("GITGOV_REF")?)?;
    let mut commit = reference.peel_to_commit()?;

    let mut update_repo = UpdateRepo::new(UPDATE_REPO_BASE)?;
    let mut doc_repo = DocRepo::new(DOC_REPO_BASE)?;

    loop {
        if commit.author().email().unwrap() == "info@gov.uk" {
            let extractor = Extractor {
                commit: &commit,
                repo: &repo,
            };
            import_docs_from_commit(&extractor, &mut doc_repo)
                .context(format!("Importing docs from {}", commit.id()))?;
            if let Err(error) = import_updates_from_commit(&extractor, &mut update_repo) {
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

fn import_updates_from_commit(extractor: &Extractor, update_repo: &mut UpdateRepo) -> Result<()> {
    // println!("import {}", extractor.commit.message().unwrap());
    let url = extractor.url()?;
    let ts = extractor.timestamp()?;
    let change = extractor.message()?;
    let _tag = extractor.tag()?;
    match update_repo.create(url.clone(), ts, change) {
        Ok((update, _events)) => {
            println!("create {}", &update);
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            let existing = update_repo.get_update(url, ts)?;
            if existing.change() == change {
                println!("exists {}", &existing);
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

fn import_docs_from_commit(extractor: &Extractor, doc_repo: &mut DocRepo) -> Result<()> {
    // println!("import {}", extractor.commit.message().unwrap());
    let docs = extractor.doc_versions().context("loading doc versions")?;
    let ts = extractor.timestamp()?;
    let _tag = extractor.tag()?;
    for (url, blob) in docs {
        let content = normalise(blob.content())?;
        match doc_repo.create(url.clone(), ts) {
            Ok(mut writer) => {
                writer.write_all(content.as_bytes())?;
                let (update, _events) = writer.done()?;
                println!("create {}", &update);
                Ok(())
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                let existing = doc_repo.ensure_version(url.clone(), ts)?;
                let mut data: Vec<u8> = vec![];
                doc_repo.open(&existing)?.read_to_end(&mut data)?;
                if data == content.as_bytes() {
                    println!("exists {}", &existing);
                    Ok(())
                } else {
                    // let diff = html_diff::get_differences(from_utf8(&data)?, from_utf8(blob.content())?); // TODO pre strip test data
                    let existing = normalise(blob.content())?;
                    let diff = prettydiff::diff_lines(from_utf8(&data)?, &existing);
                    Err(format_err!(
                        "Update exists for {}/{} with different content : {}",
                        &url.as_str(),
                        &ts,
                        diff,
                    ))
                }
            }
            Err(err) => Err(err).context("error writing update"),
        }?;
    }
    Ok(())
}

fn normalise(input: &[u8]) -> Result<String> {
    let html = scraper::Html::parse_fragment(std::str::from_utf8(input)?);
    let root = html.root_element();
    let traversal_scope = if root.value().name.local == html5ever::local_name!("html") {
        TraversalScope::ChildrenOnly(None)
    } else {
        TraversalScope::IncludeNode
    };
    let opts = SerializeOpts {
        scripting_enabled: false, // It's not clear what this does.
        traversal_scope,
        create_missing_parent: false,
    };
    let mut buf = Vec::new();
    let mut ser = NormalizingHtmlSerializer(HtmlSerializer::new(&mut buf, opts.clone()));
    root.serialize(&mut ser, opts.traversal_scope)?;
    Ok(String::from_utf8(buf).unwrap())
}

struct NormalizingHtmlSerializer<Wr: Write>(HtmlSerializer<Wr>);

impl<Wr: Write> Serializer for NormalizingHtmlSerializer<Wr> {
    fn start_elem<'a, AttrIter>(&mut self, name: html5ever::QualName, attrs: AttrIter) -> io::Result<()>
    where
        AttrIter: Iterator<Item = html5ever::serialize::AttrRef<'a>>,
    {
        let mut attrs: Vec<_> = attrs.collect();
        attrs.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));
        self.0.start_elem(name, attrs.into_iter())
    }

    fn end_elem(&mut self, name: html5ever::QualName) -> io::Result<()> {
        self.0.end_elem(name)
    }

    fn write_text(&mut self, text: &str) -> io::Result<()> {
        self.0.write_text(text)
    }

    fn write_comment(&mut self, text: &str) -> io::Result<()> {
        self.0.write_comment(text)
    }

    fn write_doctype(&mut self, name: &str) -> io::Result<()> {
        self.0.write_doctype(name)
    }

    fn write_processing_instruction(&mut self, target: &str, data: &str) -> io::Result<()> {
        self.0.write_processing_instruction(target, data)
    }
}

#[test]
fn test_normalise_html() {
    assert_eq!(&normalise(br#"<div class="foo" id="bar"></div>"#).unwrap(), &normalise(br#"<div id="bar" class="foo"></div>"#).unwrap());
}

struct Extractor<'r> {
    repo: &'r git2::Repository,
    commit: &'r git2::Commit<'r>,
}

impl<'r> Extractor<'r> {
    fn diff(&self) -> Result<Diff> {
        let tree = self.commit.tree()?;
        let parent_tree = self.commit.parents().next().as_ref().map(Commit::tree).transpose()?;
        Ok(self.repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?)
    }

    fn doc_versions(&self) -> Result<Vec<(Url, Blob)>> {
        let mut removed_paths = HashSet::new();
        let mut added_paths = HashSet::new();
        let mut v = vec![];
        for diff in self.diff()?.deltas() {
            let file = diff.new_file();
            let mut path = file.path().unwrap().to_owned();
            if file.id() == Oid::zero() {
                removed_paths.insert(path.to_owned());
                eprintln!("Skipping deleted file, TODO watch for renames and normalise filenames");
                continue;
            }
            let blob =
                self.repo
                    .find_blob(file.id())
                    .context(format!("finding blob {} at path {:?}", file.id(), path))?;

            if path.extension().is_none() {
                let content = std::str::from_utf8(blob.content())
                    .context("failed attempting to detect filetype of blob without extension")?;
                if content.trim_start().starts_with('<') {
                    eprintln!(
                        "Inferred a .html extension for file {:?}, with content {}..",
                        path,
                        &content.trim_start()[0..10]
                    );
                    path.set_extension("html");
                } else {
                    bail!(
                        "Couldn't infer extension for file {:?}, starting with content {}..",
                        path,
                        &content[0..10]
                    );
                }
            }

            if diff.old_file().id() == Oid::zero() {
                added_paths.insert(path.to_owned());
            }

            let url = Url::from_str(&format!("https://www.gov.uk/{}", path.to_str().unwrap()))?;
            v.push((url, blob));
        }
        for removed_path in removed_paths {
            ensure!(
                added_paths.contains(&removed_path) || added_paths.contains(&removed_path.with_extension("html")),
                "{} : removed path {:?} not matched in added paths {:?}",
                self.commit.id(),
                removed_path,
                added_paths
            );
        }
        Ok(v)
    }

    fn url(&self) -> Result<Url> {
        let diff = self.diff()?;
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
        let date = self.commit.message().unwrap().split(": ").next().unwrap();
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
