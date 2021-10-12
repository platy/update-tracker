use std::{io, iter::empty, str::FromStr};

use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, FixedOffset, Offset, TimeZone, Timelike};
use chrono_tz::Tz;
use git2::{Blob, Commit, Diff, Oid};
use html5ever::serialize::{HtmlSerializer, Serialize, SerializeOpts, Serializer, TraversalScope};
use io::Write;
use lazy_static::lazy_static;
use scraper::Html;

use url::Url;
pub struct Extractor<'r> {
    repo: &'r git2::Repository,
    commit: &'r git2::Commit<'r>,
}

impl<'r> Extractor<'r> {
    pub fn new(repo: &'r git2::Repository, commit: &'r git2::Commit<'r>) -> Self {
        Extractor { repo, commit }
    }

    pub fn diff(&self) -> Result<Diff> {
        let tree = self.commit.tree()?;
        let parent_tree = self.commit.parents().next().as_ref().map(Commit::tree).transpose()?;
        Ok(self.repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?)
    }

    /// Extracts the documents contained in this commit as well as a count of skipped deleted files
    pub fn doc_versions(&self) -> Result<(Vec<(Url, DocExtractor<'_>)>, u16)> {
        let mut skip_delete = 0;
        let mut v = vec![];
        for diff in self.diff()?.deltas() {
            let file = diff.new_file();
            let path = file.path().unwrap().to_owned();
            if file.id() == Oid::zero() {
                // Deleted file means nothing, it was due to a couple of bugs (old version of fetcher recorded files with the url they were retrieved from which means conflicts between files and directories & new fetcher would overwrite those files with a directory)
                skip_delete += 1;
                continue;
            }
            let blob =
                self.repo
                    .find_blob(file.id())
                    .context(format!("finding blob {} at path {:?}", file.id(), path))?;

            let is_html = if let Some(extension) = path.extension() {
                extension == "html"
            } else {
                let content = std::str::from_utf8(blob.content())
                    .context("failed attempting to detect filetype of blob without extension")?;
                if content.trim_start().starts_with('<') {
                    true
                } else if content == "connection failure" {
                    // single error case
                    false
                } else {
                    bail!(
                        "Couldn't infer extension for file {:?}, starting with content {}..",
                        path,
                        &content[0..10]
                    );
                }
            };

            let content = if is_html {
                DocExtractor::from_html(blob.content())?
            } else {
                DocExtractor::Blob(blob)
            };

            let url = Url::from_str(&format!("https://www.gov.uk/{}", path.to_str().unwrap()))?;
            v.push((url, content));
        }
        Ok((v, skip_delete))
    }

    pub fn main_doc_version(&self) -> Result<(Url, DateTime<FixedOffset>)> {
        let ts = self.updated_at()?;
        let ts = ts.with_timezone(&ts.offset().fix());
        let change = self.message()?;
        let match_score = |(_, updated_at, description): &(_, DateTime<FixedOffset>, String)| {
            (updated_at.with_second(0).unwrap() == ts) as u8 + (change == *description) as u8
        };

        // easy path if there is only one doc in the commit
        if let Ok(url) = self.url() {
            return Ok((url, ts));
        }

        // if the docs have history, find the one that matches
        let (doc_versions, _) = self.doc_versions().context("loading doc versions")?;
        ensure!(
            !doc_versions.is_empty(),
            "No doc updates in commit {}",
            self.commit.id()
        );
        if let Some(max) = doc_versions
            .iter()
            .flat_map(|(url, content)| {
                let url = url.clone();
                content
                    .history()
                    .map(move |(updated_at, description)| (url.clone(), updated_at, description))
            })
            .max_by_key(match_score)
        {
            let score = match_score(&max);
            ensure!(
                doc_versions
                    .iter()
                    .flat_map(|(url, content)| {
                        let url = url.clone();
                        let max = &max;
                        content.history().filter_map(move |(updated_at, description)| {
                            let url = url.clone();
                            if match_score(&(url.clone(), updated_at, description.clone())) == score
                                && max != &(url.clone(), updated_at, description.clone())
                            {
                                Some((url, updated_at, description))
                            } else {
                                None
                            }
                        })
                    })
                    .count()
                    == 0,
                "More than one update in commit with the score {}",
                score
            );
            let (url, updated_at, _) = max;
            return Ok((url, updated_at));
        }

        // if one doc is a parent to all the others
        let (shortest_doc_url, _) = doc_versions
            .iter()
            .min_by_key(|dv| dv.0.path_segments().map_or(0, Iterator::count))
            .unwrap();
        let shortest_doc_path = shortest_doc_url.path();
        let shortest_doc_path = shortest_doc_path.strip_suffix(".html").unwrap_or(shortest_doc_path);
        if doc_versions
            .iter()
            .filter(|(url, _)| url != shortest_doc_url)
            .all(|(url, _)| url.path().starts_with(shortest_doc_path))
        {
            return Ok((shortest_doc_url.clone(), ts));
        }

        bail!(
            "No history found in commit {} for docs {:?}",
            self.commit.id(),
            doc_versions.iter().map(|(url, _)| url.to_string()).collect::<Vec<_>>()
        );
    }

    /// Gets the url of the changed file, if there is only one
    pub fn url(&self) -> Result<Url> {
        let diff = self.diff()?;
        let files: Vec<_> = diff.deltas().collect();
        if files.len() != 1 {
            bail!("Too many files in commit {}", self.commit.id());
        }
        let path = files[0].new_file().path().unwrap();
        let url = Url::from_str(&format!("https://www.gov.uk/{}", path.to_str().unwrap()))?;
        Ok(url)
    }

    /// timestamp of update
    pub fn updated_at(&self) -> Result<DateTime<Tz>> {
        let date = self.commit.message().unwrap().split(": ").next().unwrap();
        // println!("date{}", date);
        const DATE_FORMAT: &str = "%I:%M%p, %d %B %Y";
        let local_ts = chrono_tz::Europe::London
            .datetime_from_str(date, DATE_FORMAT)
            .context("parsing timestamp")?;
        Ok(local_ts)
    }

    /// timestamp of retrieval
    pub fn retrieved_at(&self) -> DateTime<FixedOffset> {
        let commit_time = self.commit.time();
        FixedOffset::east(commit_time.offset_minutes() * 60).timestamp(commit_time.seconds(), 0)
    }

    pub fn message(&self) -> Result<String> {
        let message = self.commit.message().unwrap().splitn(2, ": ").nth(1).unwrap();
        let message = message.split(" [").next().unwrap().trim();
        let message = message.replace('‘', "'").replace('’', "'");
        Ok(message)
    }

    pub fn tag(&self) -> Result<&str> {
        let message = self.commit.message().unwrap();
        let tag = message
            .split(" [")
            .nth(1)
            .context(format!("Couldn't find tag in '{}'", message))?
            .split(']')
            .next()
            .unwrap();
        Ok(tag)
    }
}

pub enum DocExtractor<'r> {
    Html(Html, String),
    Blob(Blob<'r>),
}

impl DocExtractor<'static> {
    fn from_html(input: &[u8]) -> Result<Self> {
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

        Ok(DocExtractor::Html(html, String::from_utf8(buf)?))
    }
}

impl<'r> DocExtractor<'r> {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            DocExtractor::Html(_html, string) => string.as_bytes(),
            DocExtractor::Blob(blob) => blob.content(),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            DocExtractor::Html(_html, string) => string,
            DocExtractor::Blob(blob) => std::str::from_utf8(blob.content()).expect("Invalid uf8 str"),
        }
    }

    pub fn history(&self) -> Box<dyn Iterator<Item = (DateTime<FixedOffset>, String)> + '_> {
        if let Self::Html(html, _) = self {
            Box::new(iter_history(html))
        } else {
            Box::new(empty())
        }
    }
}

lazy_static! {
    static ref UPDATE_SELECTOR: scraper::Selector =
        scraper::Selector::parse(".app-c-published-dates--history li time").unwrap();
}

/// Iterator over the history of updates in the document
/// Panics if it doesn't recognise the format
fn iter_history(doc: &scraper::Html) -> impl Iterator<Item = (DateTime<FixedOffset>, String)> + '_ {
    doc.select(&UPDATE_SELECTOR).map(|time_elem| {
        let time =
            DateTime::parse_from_rfc3339(time_elem.value().attr("datetime").expect("no datetime attribute")).unwrap();
        let sibling = time_elem // faffing around - this is bullshit
            .next_sibling()
            .expect("expected sibling of time element in history");
        let comment_node = sibling.next_sibling().unwrap_or(sibling);
        let comment = if let Some(comment_node) = comment_node.value().as_text() {
            comment_node.trim().to_string()
        } else {
            comment_node
                .children()
                .next()
                .unwrap()
                .value()
                .as_text()
                .unwrap()
                .trim()
                .to_string()
        };
        (time, comment)
    })
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
    assert_eq!(
        &DocExtractor::from_html(br#"<div class="foo" id="bar"></div>"#)
            .unwrap()
            .as_bytes(),
        &DocExtractor::from_html(br#"<div id="bar" class="foo"></div>"#)
            .unwrap()
            .as_bytes(),
    );
}
