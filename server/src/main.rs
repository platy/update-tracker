use std::{borrow::Borrow, cmp::Reverse, fmt, io};

use rouille::{router, Request, Response};
use update_tracker::update::{Update, UpdateRepo};

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
            _ => Response::html("Not found").with_status_code(404)
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

    Response::html(format!(include_str!("template.html"), UpdateList(updates.as_slice())))
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
                r#"<a href="/diff/{}/{}{}">
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
