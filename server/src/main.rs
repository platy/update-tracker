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
                        return handle_update_page(timestamp, url, &data).unwrap_or_else(Into::into)
                    }
                } else if let Some(request) = request.remove_prefix("/diff/") {
                    let url = request.url();
                    let mut split = url.splitn(3, '/');
                    if let (Some(from), Some(to), Some(url)) = (split.next(), split.next(), split.next()) {
                        return handle_doc_diff_page(from, to, url, &data).unwrap_or_else(Into::into)
                    }
                }
                Response::html("Not found").with_status_code(404)
            }
        )
    });
}

fn handle_updates(_request: &Request, data: &Data) -> Response {
    let updates = data.list_updates();

    Response::html(format!(include_str!("updates.html"), UpdateList(updates.as_slice())))
}

fn handle_update_page(timestamp: &str, url: &str, data: &Data) -> Result<Response, Error> {
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

    let (diff_url, from_ts, to_ts, body) = diff_fields(&url, previous_doc, current_doc, data);

    Ok(Response::html(format!(
        include_str!("update.html"),
        orig_url = &url,
        timestamp = update.timestamp(),
        change = update.change(),
        diff_url = diff_url,
        doc_from = from_ts.map_or(String::new(), |v| v.to_string()),
        doc_to = to_ts.map_or(String::new(), |v| v.to_string()),
        body = body,
    )))
}

fn handle_doc_diff_page(from: &str, to: &str, url: &str, data: &Data) -> Result<Response, Error> {
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

    let (diff_url, from_ts, to_ts, body) = diff_fields(&url, from_doc, to_doc, data);

    Ok(Response::html(format!(
        include_str!("diff.html"),
        orig_url = &url,
        diff_url = diff_url,
        doc_from = from_ts.map_or(String::new(), |v| v.to_string()),
        doc_to = to_ts.map_or(String::new(), |v| v.to_string()),
        body = body,
    )))
}

fn diff_fields(
    url: &Url,
    from: Option<DocumentVersion>,
    to: Option<DocumentVersion>,
    data: &Data,
) -> (
    String,
    Option<DateTime<FixedOffset>>,
    Option<DateTime<FixedOffset>>,
    String,
) {
    let diff_base = format!(
        "/diff/{}/{}/{}",
        from.as_ref().map_or(String::new(), |v| v.timestamp().to_rfc3339()),
        to.as_ref().map_or(String::new(), |v| v.timestamp().to_rfc3339()),
        url.host().unwrap(),
    );

    let current_doc_body = to
        .as_ref()
        .map(|doc| data.read_doc_to_string(doc).with_base_url(&diff_base));
    let previous_doc_body = from
        .as_ref()
        .map(|doc| data.read_doc_to_string(doc).with_base_url(&diff_base));

    (
        format!("{}{}", diff_base, url.path()),
        from.as_ref().map(DocumentVersion::timestamp).copied(),
        to.as_ref().map(DocumentVersion::timestamp).copied(),
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

struct UpdateList<'a, U>(&'a [U]);

impl<'a, U: Borrow<Update>> fmt::Display for UpdateList<'a, U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut current_date = None;
        for update in self.0 {
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
                <li class="added">{}</li>
            </ul>
            <p>{} {} [TODO: TAGS]</p>
        </a>"#,
                update.timestamp().to_rfc3339(),
                update.url().host_str().unwrap_or_default(),
                update.url().path(),
                update.url().path(),
                update.timestamp().time().to_string(),
                update.change(),
            )
            .unwrap();
        }
        Ok(())
    }
}
