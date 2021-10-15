use std::{borrow::Borrow, fmt, str::FromStr};

use chrono::{DateTime, FixedOffset};
use rouille::{router, Request, Response};
use update_repo::{doc::DocumentVersion, update::Update, Url};

mod data;
mod error;

use data::Data;

use crate::error::{CouldFind, Error};

const LISTEN_ADDR: &str = "localhost:8080";

fn main() {
    println!("Loading data");

    let data = Data::new();

    println!("Listen on http://{}", LISTEN_ADDR);

    rouille::start_server_with_pool(LISTEN_ADDR, None, move |request| {
        {
            let assets_match = rouille::match_assets(request, "./static");
            if assets_match.is_success() {
                return assets_match;
            }
        }
        router!(request,
            (GET) (/) => {
                Response::redirect_302("/updates")
            },
            (GET) (/updates) => {
                handle_updates(request, &data)
            },
            _ => {
                if let Some(request) = request.remove_prefix("/update/") {
                    if let Some((timestamp, url)) = request.url().split_once('/') {
                        return handle_update_page(&request, timestamp, url, &data).unwrap_or_else(Into::into)
                    }
                } else if let Some(request) = request.remove_prefix("/diff/") {
                    let url = request.url();
                    let mut split = url.splitn(3, '/');
                    if let (Some(from), Some(to), Some(url)) = (split.next(), split.next(), split.next()) {
                        return handle_doc_diff_page(&request, from, to, url, &data).unwrap_or_else(Into::into)
                    }
                }
                Response::html("Not found").with_status_code(404)
            }
        )
    });
}

fn handle_updates(request: &Request, data: &Data) -> Response {
    let updates = data.list_updates();

    let mut results = UpdateList::new(updates.iter().copied().rev(), request, data);
    let etag = results.etag();
    Response::html(format!(include_str!("updates.html"), results)).with_etag(request, etag)
}

fn handle_update_page(request: &Request, timestamp: &str, url: &str, data: &Data) -> Result<Response, Error> {
    let timestamp: DateTime<FixedOffset> = timestamp.parse().or(Err(Error::NotFound("Update")))?;
    let url: Url = format!("https://{}", url).parse().unwrap();

    // get update
    let update = data.get_update(&url, timestamp).could_find("Update")?;

    // get doc version before & after update
    let current_doc = data.iter_doc_versions(&url).and_then(|iter| {
        iter.filter(|v| v.timestamp() > &timestamp)
            .min_by_key(|v| *v.timestamp())
    });
    let previous_doc = data.iter_doc_versions(&url).and_then(|iter| {
        iter.filter(|v| v.timestamp() < current_doc.as_ref().map_or(&timestamp, DocumentVersion::timestamp))
            .max_by_key(|v| *v.timestamp())
    });

    let (diff_url, from_ts, to_ts, body) = diff_fields(&url, previous_doc.as_ref(), current_doc.as_ref(), data);

    Ok(Response::html(format!(
        include_str!("update.html"),
        orig_url = &url,
        timestamp = update.timestamp(),
        change = update.change(),
        diff_url = diff_url,
        doc_from = from_ts.map_or(String::new(), |v| v.to_string()),
        doc_to = to_ts.map_or(String::new(), |v| v.to_string()),
        body = body,
    ))
    .with_etag(request, format!("{} {}", previous_doc.is_some(), current_doc.is_some())))
}

fn handle_doc_diff_page(request: &Request, from: &str, to: &str, url: &str, data: &Data) -> Result<Response, Error> {
    let url: Url = format!("https://{}", url).parse().unwrap();

    // get doc version from
    let from = from
        .parse::<MaybeEmpty<DateTime<FixedOffset>>>()
        .or(Err(Error::NotFound("Doc")))?
        .0;
    let from_doc = from.and_then(|ts| data.get_doc_version(&url, ts).ok());

    // get doc version to
    let to = to
        .parse::<MaybeEmpty<DateTime<FixedOffset>>>()
        .or(Err(Error::NotFound("Doc")))?
        .0;
    let to_doc = to.and_then(|ts| data.get_doc_version(&url, ts).ok());

    let (diff_url, from_ts, to_ts, body) = diff_fields(&url, from_doc.as_ref(), to_doc.as_ref(), data);

    Ok(Response::html(format!(
        include_str!("diff.html"),
        orig_url = &url,
        diff_url = diff_url,
        doc_from = from_ts.map_or(String::new(), |v| v.to_string()),
        doc_to = to_ts.map_or(String::new(), |v| v.to_string()),
        body = body,
    ))
    .with_etag(request, format!("{} {}", from_doc.is_some(), to_doc.is_some())))
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

/// A paginated list of updates which can be displayed as html
struct UpdateList<'a, 'd, Us: Iterator<Item = &'a Update>> {
    data: &'d Data,
    items: std::iter::Peekable<std::iter::Take<std::iter::Skip<Us>>>,
    page_num: usize,
    page_count: usize,
    next_offset: Option<usize>,
    prev_offset: Option<usize>,
    href: String,
    offset: usize,
    filtered_count: usize,
}

impl<'a, 'd, Us: Iterator<Item = &'a Update>> UpdateList<'a, 'd, Us> {
    fn new(items: impl IntoIterator<IntoIter = Us>, request: &Request, data: &'d Data) -> Self {
        let offset = request
            .get_param("offset")
            .and_then(|offset| offset.parse().ok())
            .unwrap_or(0);
        let limit = request
            .get_param("limit")
            .and_then(|limit| limit.parse().ok())
            .unwrap_or(200);

        let items = items.into_iter();
        let filtered_count = items.size_hint().0; // should require `TrustedLen`
        let items = items.skip(offset).take(limit).peekable();

        let page_num = offset / limit + 1;
        let page_count = filtered_count / limit + 1;

        let existing_pairs = request.raw_query_string().to_owned();
        let mut href = form_urlencoded::Serializer::new(request.url() + "?");
        for (name, value) in form_urlencoded::parse(existing_pairs.as_bytes()) {
            if name != "offset" {
                href.append_pair(&name, &value);
            }
        }
        let href = href.finish();

        Self {
            data,
            items,
            page_num,
            page_count,
            prev_offset: (offset > 0).then(|| offset.checked_sub(limit).unwrap_or_default()),
            next_offset: (offset + limit <= filtered_count).then(|| offset + limit),
            href,
            offset,
            filtered_count,
        }
    }
}

impl<'a, 'd, Us: Iterator<Item = &'a Update>> UpdateList<'a, 'd, Us> {
    fn etag(&mut self) -> String {
        self.items
            .peek()
            .map_or(String::new(), |u| format!("{}", u.timestamp()))
    }
}

impl<'a, 'd, Us: Iterator<Item = &'a Update> + Clone> fmt::Display for UpdateList<'a, 'd, Us> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

        let mut count_on_page = 0;
        for update in self.items.clone() {
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
            count_on_page += 1;
        }

        writeln!(
            f,
            "</ol>
        <div>"
        )?;
        if let Some(prev_offset) = self.prev_offset {
            writeln!(
                f,
                r#"<a href="{href}&offset={prev_offset}">prev</a>"#,
                href = self.href,
                prev_offset = prev_offset,
            )?;
        }
        writeln!(
            f,
            r#" Page {page_num} of {page_count} (Updates {offset} to {last} of {total}) "#,
            page_num = self.page_num,
            page_count = self.page_count,
            offset = self.offset + 1,
            last = self.offset + count_on_page,
            total = self.filtered_count,
        )?;
        if let Some(next_offset) = self.next_offset {
            writeln!(
                f,
                r#"<a href="{href}&offset={next_offset}">next</a>"#,
                href = self.href,
                next_offset = next_offset,
            )?;
        }
        writeln!(f, "</div>")?;
        Ok(())
    }
}
