use std::{borrow::Borrow, fmt};

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

    // get doc version after update
    let current_doc = data.iter_doc_versions(&url).and_then(|iter| {
        iter.filter(|v| v.timestamp() > &timestamp)
            .min_by_key(|v| *v.timestamp())
    });
    let current_doc_body = current_doc
        .as_ref()
        .map(|current_doc| data.read_doc_to_string(current_doc));

    // get doc version before update
    let previous_doc = data.iter_doc_versions(&url).and_then(|iter| {
        iter.filter(|v| v.timestamp() < current_doc.as_ref().map_or(&timestamp, DocumentVersion::timestamp))
            .max_by_key(|v| *v.timestamp())
    });
    let previous_doc_body = previous_doc
        .as_ref()
        .map(|previous_doc| data.read_doc_to_string(previous_doc));

    Ok(Response::html(format!(
        include_str!("update.html"),
        update.url(),
        update.url(),
        update.timestamp(),
        update.change(),
        previous_doc
            .as_ref()
            .map_or(String::new(), |v| v.timestamp().to_string()),
        current_doc
            .as_ref()
            .map_or(String::new(), |v| v.timestamp().to_string()),
        match (previous_doc_body, current_doc_body) {
            (Some(previous_doc_body), Some(current_doc_body)) =>
                htmldiff::htmldiff(&previous_doc_body, &current_doc_body),
            (Some(previous_doc_body), None) => previous_doc_body,
            (None, Some(current_doc_body)) => current_doc_body,
            _ => "No document recorded for this update".to_owned(),
        },
    )))
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
                update.url().as_ref().host_str().unwrap_or_default(),
                update.url().as_ref().path(),
                update.url().as_ref().path(),
                update.timestamp().time().to_string(),
                update.change(),
            )
            .unwrap();
        }
        Ok(())
    }
}
