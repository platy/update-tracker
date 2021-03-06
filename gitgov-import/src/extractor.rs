use std::{io, iter::empty, str::FromStr};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use git2::{Blob, Commit, Diff, Oid};
use html5ever::serialize::{HtmlSerializer, Serialize, SerializeOpts, Serializer, TraversalScope};
use io::Write;
use scraper::{Html};

use update_tracker::doc::iter_history;
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

    pub fn doc_versions(&self) -> Result<Vec<(Url, DocExtractor<'_>)>> {
        let mut v = vec![];
        for diff in self.diff()?.deltas() {
            let file = diff.new_file();
            let path = file.path().unwrap().to_owned();
            if file.id() == Oid::zero() {
                eprintln!(
                    "Deleted file means nothing, it was due to a couple of bugs (old version of fetcher recorded files with the url they were retrieved from which means conflicts between files and directories & new fetcher would overwrite those files with a directory) : {:?}",
                    path
                );
                continue;
            }
            let blob =
                self.repo
                    .find_blob(file.id())
                    .context(format!("finding blob {} at path {:?}", file.id(), path))?;

            let is_html = if let Some(extension) = path.extension() { extension == "html" }
            else {
                let content = std::str::from_utf8(blob.content())
                    .context("failed attempting to detect filetype of blob without extension")?;
                if content.trim_start().starts_with('<') {
                    true
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
        Ok(v)
    }

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
    pub fn updated_at(&self) -> Result<DateTime<Utc>> {
        let date = self.commit.message().unwrap().split(": ").next().unwrap();
        // println!("date{}", date);
        const DATE_FORMAT: &str = "%I:%M%p, %d %B %Y";
        let local_ts = chrono_tz::Europe::London.datetime_from_str(date, DATE_FORMAT).context("parsing timestamp")?;
        Ok(local_ts.with_timezone(&Utc))
    }

    /// timestamp of retrieval
    pub fn retrieved_at(&self) -> DateTime<Utc> {
        let commit_time = self.commit.time();
        FixedOffset::east(commit_time.offset_minutes() * 60)
            .timestamp(commit_time.seconds(), 0)
            .into()
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
            DocExtractor::Html(_html, string) => &string,
            DocExtractor::Blob(blob) => std::str::from_utf8(blob.content()).expect("Invalid uf8 str"),
        }
    }

    pub fn history(&self) -> Box<dyn Iterator<Item = (DateTime<Utc>, String)> + '_> {
        if let Self::Html(html, _) = self {
            Box::new(iter_history(html))
        } else {
            Box::new(empty())
        }
    }
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
