use std::{
    borrow::Borrow,
    fmt::{self, Write},
    mem,
    ops::Deref,
    str::FromStr,
};

use chrono::{DateTime, FixedOffset};
use rouille::{find_route, Request, Response};
use update_repo::{doc::DocumentVersion, tag::Tag, update::Update, Url};

#[macro_use]
mod web_macros;
mod error;
mod page;

use crate::data::Data;

use error::{CouldFind, Error};

pub(crate) fn listen(addr: &str, data: Data) {
    println!("Loading data");

    println!("Listen on http://{}", addr);

    rouille::start_server_with_pool(addr, None, move |request| {
        find_route!(
            rouille::match_assets(request, "./static"),
            handle_root(request),
            handle_updates(request, &data),
            handle_update(request, &data),
            handle_doc_diff_page(request, &data)
        )
    });
}

route! {
    (GET /)
    handle_root(request: &Request) {
        Ok(Response::redirect_302("/updates"))
    }
}

route! {
    (GET /updates)
    handle_updates(request: &Request, data: &Data) {
        let url_prefix = request.get_param("url_prefix").as_deref().unwrap_or("www.gov.uk/").parse::<HttpsStrippedUrl>().map_err(|_| Error::InvalidRequest)?.0;
        let change_terms = request.get_param("change").filter(|t| !t.is_empty());
        let tag = request.get_param("tag").filter(|t| !t.is_empty()).map(Tag::new);

        let updates = data.list_updates(&url_prefix, change_terms, tag);

        Ok(updates_page_response(updates,request,data))
    }
}

route! {
    (GET /update/{timestamp: DateTime<FixedOffset>}/{url: HttpsStrippedUrl})
    handle_update(request: &Request, data: &Data) {
        // get update
        let updates = data.get_updates(&url).could_find("Update")?;
        let update = &updates.get(&timestamp).could_find("Update")?.0;

        // get doc version before & after update
        let current_doc = data.iter_doc_versions(&url).and_then(|iter| {
            iter.filter(|v| v.timestamp() > &timestamp)
                .min_by_key(|v| *v.timestamp())
        });
        let previous_doc = data.iter_doc_versions(&url).and_then(|iter| {
            iter.filter(|v| v.timestamp() < current_doc.as_ref().map_or(&timestamp, DocumentVersion::timestamp))
                .max_by_key(|v| *v.timestamp())
        });

        // do the diff
        let (diff_url, from_ts, to_ts, body) = diff_fields(&url, previous_doc.as_ref(), current_doc.as_ref(), data);

        Ok(Response::html(format!(
            include_str!("update.html"),
            orig_url = &*url,
            timestamp = update.timestamp().naive_local(),
            change = update.change(),
            diff_url = diff_url,
            doc_from = from_ts.map_or(String::new(), |v| v.to_string()),
            doc_to = to_ts.map_or(String::new(), |v| v.to_string()),
            body = body,
            history = updates.iter().rev().map(|(_, (update, _tags))| {
                format!(r#"<a href="/update/{}/{}{}"><p>{}<br />{}</p></a>"#, update.timestamp().to_rfc3339(), update.url().host_str().unwrap(), update.url().path(), update.timestamp().naive_local(), update.change())
            }).collect::<String>()
        ))
        .with_etag(
            request,
            format!("{} {}", previous_doc.is_some(), current_doc.is_some()),
        ))
    }
}

route! {
    (GET /diff/{from: MaybeEmpty<DateTime<FixedOffset>>}/{to: MaybeEmpty<DateTime<FixedOffset>>}/{url: HttpsStrippedUrl})
    handle_doc_diff_page(request: &Request, data: &Data) {
        // get doc version from
        let from_doc = from.0.and_then(|ts| data.get_doc_version(&url, ts).ok());

        // get doc version to
        let to_doc = to.0.and_then(|ts| data.get_doc_version(&url, ts).ok());

        // do the diff
        let (diff_url, from_ts, to_ts, body) = diff_fields(&url, from_doc.as_ref(), to_doc.as_ref(), data);

        Ok(Response::html(format!(
            include_str!("diff.html"),
            orig_url = &*url,
            diff_url = diff_url,
            doc_from = from_ts.map_or(String::new(), |v| v.to_string()),
            doc_to = to_ts.map_or(String::new(), |v| v.to_string()),
            body = body,
        ))
        .with_etag(request, format!("{} {}", from_doc.is_some(), to_doc.is_some())))
    }
}

fn updates_page_response<'a>(updates: impl Iterator<Item = &'a Update>, request: &Request, data: &Data) -> Response {
    let mut results = UpdateList::new(updates, request, data);
    let etag = results.etag();
    let mut result_string = String::new(); // ugh
    results.into_writer(&mut result_string).unwrap();
    let selected_tag = request.get_param("tag");
    Response::html(format!(
        include_str!("updates.html"),
        result_string,
        url_prefix_filter = request.get_param("url_prefix").as_deref().unwrap_or("www.gov.uk/"),
        change_filter = request.get_param("change").as_deref().unwrap_or(""),
        tag_options = data
            .all_tags()
            .map(|tag| format!(
                r#"<option {selected}>{tag}</option>"#,
                tag = tag,
                selected = (selected_tag.as_ref() == Some(tag))
                    .then(|| "selected")
                    .unwrap_or_default()
            ))
            .collect::<String>()
    ))
    .with_etag(request, etag)
}

fn diff_fields(
    url: &Url,
    from: Option<&DocumentVersion>,
    to: Option<&DocumentVersion>,
    data: &Data,
) -> (
    String,
    Option<DateTime<FixedOffset>>,
    Option<DateTime<FixedOffset>>,
    String,
) {
    let diff_base = format!(
        "/diff/{}/{}/{}",
        from.map_or(String::new(), |v| v.timestamp().to_rfc3339()),
        to.map_or(String::new(), |v| v.timestamp().to_rfc3339()),
        url.host().unwrap(),
    );

    let current_doc_body = to.map(|doc| data.read_doc_to_string(doc).with_base_url(&diff_base));
    let previous_doc_body = from.map(|doc| data.read_doc_to_string(doc).with_base_url(&diff_base));

    (
        format!("{}{}", diff_base, url.path()),
        from.map(DocumentVersion::timestamp).copied(),
        to.map(DocumentVersion::timestamp).copied(),
        match (previous_doc_body, current_doc_body) {
            (Some(previous_doc_body), Some(current_doc_body)) => previous_doc_body.diff(&current_doc_body),
            (Some(previous_doc_body), None) => previous_doc_body.into_inner(),
            (None, Some(current_doc_body)) => current_doc_body.into_inner(),
            _ => "No versions recorded for this update".to_owned(),
        },
    )
}

/// Parse helper for deserialising things where an empty string means `None`
struct MaybeEmpty<T>(Option<T>);

impl<T: FromStr> FromStr for MaybeEmpty<T> {
    type Err = T::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            Ok(Self(None))
        } else {
            Ok(Self(Some(s.parse()?)))
        }
    }
}

impl<T> Deref for MaybeEmpty<T> {
    type Target = Option<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Parse helper for deserialising a url where 'https://' is elided and implied
struct HttpsStrippedUrl(Url);

impl FromStr for HttpsStrippedUrl {
    type Err = url::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(HttpsStrippedUrl(
            url::Url::parse(&format!("https://{}", s)).unwrap().into(),
        ))
    }
}

impl Deref for HttpsStrippedUrl {
    type Target = Url;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A paginated list of updates which can be displayed as html
struct UpdateList<'a, 'd, Us: Iterator<Item = &'a Update>> {
    data: &'d Data,
    page: page::Page<std::iter::Peekable<Us>>,
    etag: String,
}

impl<'a, 'd, Us: Iterator<Item = &'a Update>> UpdateList<'a, 'd, Us> {
    fn new(items: impl IntoIterator<IntoIter = Us>, request: &Request, data: &'d Data) -> Self {
        let mut items = items.into_iter().peekable();
        Self {
            data,
            etag: items.peek().map_or(String::new(), |u| format!("{}", u.timestamp())),
            page: page::Page::new(request, items),
        }
    }

    fn into_writer(mut self, f: &mut String) -> fmt::Result {
        let mut current_date = None;
        writeln!(
            f,
            r#"
    <ol class="commit-log">
        <div class="table-header">
            <div>Filename on gov.uk</div>
            <div>Change description</div>
            <div>Tags</div>
        </div>"#
        )?;

        for update in &mut self.page {
            let update = update.borrow();
            let update_date = update.timestamp().date();
            if Some(update_date) != current_date {
                current_date = Some(update_date);
                writeln!(f, "<h3>{}</h3>", update_date.naive_local()).unwrap();
            }
            writeln!(
                f,
                r#"<a href="/update/{}/{}{}">
            <ul>
                <li>{}</li>
            </ul>
            <p>{} {}</p>
            <ul class="tags">
                {tags}
            </ul>
        </a>"#,
                update.timestamp().to_rfc3339(),
                update.url().host_str().unwrap_or_default(),
                update.url().path(),
                update.url().path(),
                update.timestamp().time().to_string(),
                update.change(),
                tags = self
                    .data
                    .get_tags(update.update_ref())
                    .iter()
                    .map(|t| format!("<li>{}</li>", t.name()))
                    .collect::<String>(),
            )
            .unwrap();
        }

        writeln!(
            f,
            "</ol>
        <div>"
        )?;
        self.page.into_writer(f)?;
        Ok(())
    }
}

impl<'a, 'd, Us: Iterator<Item = &'a Update>> UpdateList<'a, 'd, Us> {
    fn etag(&mut self) -> String {
        mem::take(&mut self.etag)
    }
}
