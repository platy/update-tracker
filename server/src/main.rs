use std::{
    borrow::Borrow,
    cmp::Reverse,
    fmt,
    io::{self, Read},
};

use chrono::{DateTime, FixedOffset};
use rouille::{router, Request, Response};
use update_tracker::{
    doc::DocRepo,
    update::{Update, UpdateRepo},
    Url,
};

fn main() {
    const LISTEN_ADDR: &str = "localhost:8080";
    println!("Listen on http://{}", LISTEN_ADDR);

    rouille::start_server_with_pool(LISTEN_ADDR, None, |request| {
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
                handle_updates(request)
            },
            _ => {
                if let Some(request) = request.remove_prefix("/update/") {
                    if let Some((timestamp, url)) = request.url().split_once('/') {
                        let timestamp: DateTime::<FixedOffset> = timestamp.parse().unwrap();
                        let url: Url = format!("https://{}", url).parse().unwrap();
                        return handle_update_page(timestamp, url)
                    }
                }
                Response::html("Not found").with_status_code(404)
            }
        )
    });
}

fn handle_updates(_request: &Request) -> Response {
    let update_repo = UpdateRepo::new("../repo/url").unwrap();
    let mut updates: Vec<_> = update_repo
        .list_all(&"https://www.gov.uk/".parse().unwrap())
        .unwrap()
        .collect::<io::Result<_>>()
        .unwrap();
    updates.sort_by_key(|u| Reverse(u.timestamp().to_owned()));

    Response::html(format!(include_str!("updates.html"), UpdateList(updates.as_slice())))
}

fn handle_update_page(timestamp: DateTime<FixedOffset>, url: Url) -> Response {
    let update_repo = UpdateRepo::new("../repo/url").unwrap();
    let update = update_repo.get_update(url.clone(), timestamp).unwrap();
    let doc_repo = DocRepo::new("../repo/url").unwrap();
    let current_doc = doc_repo
        .list_versions(url.clone())
        .unwrap()
        .filter_map(|r| r.ok())
        .filter(|v| v.timestamp() > &timestamp)
        .min_by_key(|v| *v.timestamp())
        .unwrap();
    let mut current_doc_body = String::new();
    doc_repo
        .open(&current_doc)
        .unwrap()
        .read_to_string(&mut current_doc_body)
        .unwrap(); // TODO handle none
    let previous_doc = doc_repo
        .list_versions(url)
        .unwrap()
        .filter_map(|r| r.ok())
        .filter(|v| v.timestamp() < current_doc.timestamp())
        .max_by_key(|v| *v.timestamp())
        .unwrap();
    let mut previous_doc_body = String::new();
    doc_repo
        .open(&previous_doc)
        .unwrap()
        .read_to_string(&mut previous_doc_body)
        .unwrap(); // TODO handle none
    // TODO html diff
    Response::html(format!(
        include_str!("update.html"),
        update.url(),
        update.url(),
        update.timestamp(),
        update.change(),
        previous_doc.timestamp(),
        current_doc.timestamp(),
        current_doc_body
    ))
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
