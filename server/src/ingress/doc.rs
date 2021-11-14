use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use html5ever::{
    serialize::{SerializeOpts, TraversalScope},
    Attribute,
    ParseOpts,
};
use html5streams::{HtmlPath, HtmlPathElement, HtmlSerializer, HtmlSink};
use scraper::{Html, Selector};
use url::Url;

#[derive(Debug, Eq, PartialEq)]
pub struct Doc {
    pub url: Url,
    pub content: DocContent,
}

#[derive(Debug, Eq, PartialEq)]
pub enum DocContent {
    DiffableHtml(String, Vec<Url>, Vec<DocUpdate>),
    Other(Vec<u8>),
}

#[derive(Debug, Eq, PartialEq)]
pub struct DocUpdate(DateTime<Utc>, String);

impl DocContent {
    pub fn html(html: &str, url: Option<&Url>) -> Result<Self> {
        let main_selector: Selector = Selector::parse("main").unwrap();
        let history_selector: Selector = Selector::parse("#full-history li").unwrap();
        let time_selector: Selector = Selector::parse("time").unwrap();
        let p_selector: Selector = Selector::parse("p").unwrap();

        let html = Html::parse_document(html);

        let main = html.select(&main_selector).next().context("No main found")?;
        let mut history = vec![];
        for history_elem in html.select(&history_selector) {
            let time_elem = history_elem.select(&time_selector).next().context("No time found")?;
            history.push(DocUpdate(
                time_elem
                    .value()
                    .attr("datetime")
                    .context("Missing \"datetime\" property on time tag")?
                    .parse()?,
                history_elem
                    .select(&p_selector)
                    .next()
                    .context("Missing p tag")?
                    .inner_html(),
            ))
        }
        let mut attachments = vec![];
        for attachment_url in attachments_from(&html) {
            attachments.push(if let Some(url) = url {
                url.join(&attachment_url)?
            } else {
                attachment_url.parse()?
            });
        }
        Ok(DocContent::DiffableHtml(
            remove_ids(&main.html())?,
            attachments,
            history,
        ))
    }

    pub fn is_html(&self) -> bool {
        match self {
            Self::DiffableHtml(_, _, _) => true,
            Self::Other(_) => false,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        match self {
            DocContent::DiffableHtml(string, _, _) => string.as_bytes(),
            DocContent::Other(bytes) => bytes.as_slice(),
        }
    }

    pub fn history(&self) -> Option<&[DocUpdate]> {
        match self {
            DocContent::DiffableHtml(_, _, history) => Some(history.as_slice()),
            DocContent::Other(_) => None,
        }
    }

    pub fn attachments(&self) -> Option<&[Url]> {
        match self {
            DocContent::DiffableHtml(_, attachments, _) => Some(attachments.as_slice()),
            DocContent::Other(_) => None,
        }
    }
}

impl AsRef<[u8]> for DocContent {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl DocUpdate {
    pub fn new(date: DateTime<Utc>, summary: impl Into<String>) -> Self {
        Self(date, summary.into())
    }
}

pub struct HtmlSanitizer<InputHandle: Eq + Copy, S: HtmlSink<InputHandle>> {
    inner: S,
    skip_handle: Option<InputHandle>,
}

impl<InputHandle: Eq + Copy, S: HtmlSink<InputHandle>> HtmlSanitizer<InputHandle, S> {
    pub fn wrap(sink: S) -> Self {
        Self {
            inner: sink,
            skip_handle: None,
        }
    }
}

impl<InputHandle: Eq + Copy, S: HtmlSink<InputHandle>> HtmlSink<InputHandle> for HtmlSanitizer<InputHandle, S> {
    type Output = S::Output;

    fn append_doctype_to_document(
        &mut self,
        name: html5ever::tendril::StrTendril,
        public_id: html5ever::tendril::StrTendril,
        system_id: html5ever::tendril::StrTendril,
    ) {
        self.inner.append_doctype_to_document(name, public_id, system_id)
    }

    fn append_element(&mut self, path: HtmlPath<'_, InputHandle>, mut element: HtmlPathElement<'_, InputHandle>) {
        if let Some(skip_handle) = self.skip_handle {
            if path.iter().any(|elem| elem.handle == skip_handle) {
                return;
            } else {
                self.skip_handle = None
            }
        }
        let attrs: Vec<_> = element
            .attrs
            .iter()
            .filter(|Attribute { name, value: _ }| !["id", "aria-labelledby", "aria-hidden"].contains(&&*name.local))
            .cloned() // TODO : avoid cloning when not necessary
            .collect();
        let skip = attrs.iter().any(|Attribute { name, value }| {
            &name.local == "class"
                && value
                    .split_whitespace()
                    .any(|class| class == "gem-c-contextual-sidebar")
        });
        if skip {
            self.skip_handle = Some(element.handle);
            return;
        }
        element.attrs = attrs.into();
        self.inner.append_element(path, element)
    }

    fn append_text(&mut self, path: HtmlPath<InputHandle>, text: &str) {
        if let Some(skip_handle) = self.skip_handle {
            if path.iter().any(|elem| elem.handle == skip_handle) {
                return;
            } else {
                self.skip_handle = None
            }
        }
        self.inner.append_text(path, text)
    }

    fn reset(&mut self) -> Self::Output {
        self.skip_handle = None;
        self.inner.reset()
    }
}

pub fn remove_ids(html: &str) -> Result<String> {
    let opts = SerializeOpts {
        scripting_enabled: false,
        traversal_scope: TraversalScope::IncludeNode,
        create_missing_parent: false,
    };
    let mut buf = Vec::new();
    let mut html_serializer = HtmlSerializer::new(&mut buf, opts);
    let sink = HtmlSanitizer::wrap(&mut html_serializer);
    let mut parse_opts = ParseOpts::default();
    parse_opts.tree_builder.exact_errors = true;
    let parser = html5streams::parse_fragment(sink, parse_opts);
    html5ever::tendril::TendrilSink::one(parser, html);
    Ok(String::from_utf8(buf).unwrap())
}

fn attachments_from(html: &Html) -> Vec<String> {
    let attachment_selector = Selector::parse(".attachment .title a, .attachment .download a").unwrap();
    let attachments = html
        .select(&attachment_selector)
        .map(|el| el.value().attr("href"))
        .flatten()
        .map(ToString::to_string)
        .collect();
    attachments
}
