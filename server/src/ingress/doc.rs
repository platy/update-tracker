use std::mem;

use anyhow::{format_err, Context, Result};
use chrono::{DateTime, Utc};
use html5ever::{
    local_name,
    ns,
    serialize::{SerializeOpts, TraversalScope},
    tendril::{StrTendril, TendrilSink},
    Attribute,
    ParseOpts,
};
use html5streams::{
    css_select,
    selector::{ContextualSelector, Selector},
    HtmlContext,
    HtmlPathElement,
    HtmlSerializer,
    HtmlSink,
    RootFilter,
};
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
        let opts = SerializeOpts {
            scripting_enabled: false,
            traversal_scope: TraversalScope::IncludeNode,
            create_missing_parent: false,
        };
        // stream is main selection -> sanitiser ( -> attachment extractor ) ( -> history selector -> history extractor ) -> serializer
        let mut buf = Vec::new();
        let mut html_serializer = HtmlSerializer::new(&mut buf, opts);
        let history_selector = css_select!((#"full-history") ("li"));
        let history_extractor = RootFilter::<_, _, _, Vec<_>>::wrap(HistoryExtractor::default(), history_selector);
        let attachment_extractor = AttachmentExtractor::default();
        let sink = HtmlSanitizer::wrap(((attachment_extractor, history_extractor), &mut html_serializer));

        let mut parse_opts = ParseOpts::default();
        parse_opts.tree_builder.exact_errors = true;
        let parser = html5streams::parse_document(sink, parse_opts);

        let ((attachments, history), ()) = parser.one(html);

        let attachments: Vec<Url> = if let Some(url) = url {
            attachments
                .into_iter()
                .map(|attachment_url| url.join(&*attachment_url))
                .collect::<Result<_, _>>()?
        } else {
            attachments
                .into_iter()
                .map(|attachment_url| attachment_url.parse())
                .collect::<Result<_, _>>()?
        };
        Ok(DocContent::DiffableHtml(
            String::from_utf8(buf).unwrap(),
            attachments,
            history.into_iter().collect::<Result<_>>()?,
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
    main_handle: Option<InputHandle>,
}

impl<InputHandle: Eq + Copy, S: HtmlSink<InputHandle>> HtmlSanitizer<InputHandle, S> {
    pub fn wrap(sink: S) -> Self {
        Self {
            inner: sink,
            skip_handle: None,
            main_handle: None,
        }
    }
}

impl<InputHandle: Eq + Copy, S: HtmlSink<InputHandle>> HtmlSink<InputHandle> for HtmlSanitizer<InputHandle, S> {
    type Output = S::Output;

    fn append_doctype_to_document(
        &mut self,
        _name: &html5ever::tendril::StrTendril,
        _public_id: &html5ever::tendril::StrTendril,
        _system_id: &html5ever::tendril::StrTendril,
    ) {
    }

    fn append_element(
        &mut self,
        mut context: HtmlContext<'_, InputHandle>,
        element: &HtmlPathElement<'_, InputHandle>,
    ) {
        // select
        if let Some(select_handle) = self.main_handle {
            if let Some(select_index) = context
                .iter()
                .enumerate()
                .find_map(|(index, elem)| (elem.handle == select_handle).then(|| index))
            {
                context = &context[select_index..];
            } else {
                // select ends
                self.main_handle = None;
                return;
            }
        }
        if self.main_handle.is_none() && css_select!("main").is_match(element) {
            // select starts
            context = &[];
            let select_handle = element.handle;
            self.main_handle = Some(select_handle);
        } else if self.main_handle.is_none() {
            return;
        }

        // skip
        if let Some(skip_handle) = self.skip_handle {
            if context.iter().any(|elem| elem.handle == skip_handle) {
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
        let mut element = element.clone();
        element.attrs = attrs.into();
        self.inner.append_element(context, &element)
    }

    fn append_text(&mut self, context: HtmlContext<InputHandle>, text: &str) {
        if let Some(select_handle) = self.main_handle {
            if let Some(select_index) = context
                .iter()
                .enumerate()
                .find_map(|(index, elem)| (elem.handle == select_handle).then(|| index))
            {
                let context = &context[select_index..];
                if let Some(skip_handle) = self.skip_handle {
                    if context.iter().any(|elem| elem.handle == skip_handle) {
                        return;
                    } else {
                        self.skip_handle = None
                    }
                }
                self.inner.append_text(context, text)
            } else {
                self.main_handle = None
            }
        }
    }

    fn reset(&mut self) -> Self::Output {
        self.skip_handle = None;
        self.inner.reset()
    }
}

#[derive(Default)]
struct AttachmentExtractor(Vec<StrTendril>);

impl HtmlSink<u32> for AttachmentExtractor {
    type Output = Vec<StrTendril>;

    fn append_doctype_to_document(
        &mut self,
        _name: &html5ever::tendril::StrTendril,
        _public_id: &html5ever::tendril::StrTendril,
        _system_id: &html5ever::tendril::StrTendril,
    ) {
    }

    fn append_element(&mut self, context: HtmlContext<'_, u32>, element: &HtmlPathElement<'_, u32>) {
        use html5ever::*;

        const HREF: QualName = QualName {
            prefix: None,
            ns: ns!(),
            local: local_name!("href"),
        };
        let matcher =
            css_select!((."attachment") (."title") ("a")).or(css_select!((."attachment") (."download") ("a")));
        if matcher.context_match(context, element) {
            if let Some(href) = element.attr(HREF) {
                self.0.push(href.clone());
            }
        }
    }

    fn append_text(&mut self, _context: HtmlContext<u32>, _text: &str) {}

    fn reset(&mut self) -> Self::Output {
        mem::take(&mut self.0)
    }
}

#[derive(Default)]
struct HistoryExtractor {
    timestamp: Option<DateTime<Utc>>,
    description: String,
}

impl HtmlSink<u32> for HistoryExtractor {
    type Output = Result<DocUpdate>;

    fn append_doctype_to_document(
        &mut self,
        _name: &html5ever::tendril::StrTendril,
        _public_id: &html5ever::tendril::StrTendril,
        _system_id: &html5ever::tendril::StrTendril,
    ) {
    }

    fn append_element(&mut self, context: HtmlContext<'_, u32>, element: &HtmlPathElement<'_, u32>) {
        use html5ever::*;
        const DATETIME: QualName = QualName {
            prefix: None,
            ns: ns!(),
            local: local_name!("datetime"),
        };

        if css_select!("time").context_match(context, element) {
            self.timestamp = element
                .attr(DATETIME)
                .context("Missing \"datetime\" property on time tag")
                .unwrap()
                .parse()
                .ok();
        }
    }

    fn append_text(&mut self, context: HtmlContext<u32>, text: &str) {
        if let Some(last) = context.last() {
            if css_select!("p").context_match(&[], last) {
                self.description = text.to_owned();
            }
        }
    }

    fn reset(&mut self) -> Self::Output {
        let timestamp = self
            .timestamp
            .take()
            .ok_or(format_err!("No timestamp found for history item"))?;
        Ok(DocUpdate(timestamp, mem::take(&mut self.description)))
    }
}
