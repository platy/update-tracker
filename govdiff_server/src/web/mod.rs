use std::{
    borrow::Cow,
    env,
    fmt::{self, Write},
    mem,
    ops::Deref,
    str::FromStr,
    sync::{Arc, RwLock, RwLockWriteGuard},
    time::Instant,
};

use chrono::{format::StrftimeItems, DateTime, FixedOffset};
use rouille::{find_route, Request, Response};
use update_repo::{doc::DocumentVersion, tag::Tag, update::Update, Url};

#[macro_use]
mod web_macros;
mod error;
mod page;

use crate::data::Data;

use error::{CouldFind, Error};

pub fn listen(addr: &str, data: Arc<RwLock<Data>>) {
    println!("Loading data");

    println!("Listen on http://{}", addr);

    let default_page_fast_cache = FastCache::default();

    rouille::start_server_with_pool(addr, None, move |request| {
        let start = Instant::now();
        let response = find_route!(
            rouille::match_assets(request, "./static"),
            handle_root(request),
            handle_updates(request, &data.read().unwrap(), &default_page_fast_cache),
            handle_update(request, &data.read().unwrap()),
            handle_doc_diff_page(request, &data.read().unwrap())
        );
        eprintln!(
            "> {ts} {remote_ip:15} < {status_code:3} ({took:3.0}ms) <- {method:4} {url} [Referer: {referrer:?} User-agent: {user_agent:?}]",
            ts = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            method = request.method(),
            url = request.url(),
            status_code = response.status_code,
            remote_ip = request
                .header("X-Forwarded-For")
                .map(Cow::from)
                .unwrap_or_else(|| request.remote_addr().ip().to_string().into()),
            referrer = request.header("Referer").unwrap_or_default(),
            user_agent = request.header("User-Agent").unwrap_or_default(),
            took = Instant::now().duration_since(start).as_millis(),
        );
        response
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
    handle_updates(request: &Request, data: &Data, fast_cache: &FastCache) {
        let data_updated_at = data.updated_at();
        let cache_guard =
        if request.raw_query_string().is_empty() { // default query, use fast cache
            match fast_cache.try_cache(data_updated_at) {
                Ok((html, etag)) => return Ok(Response::html(html).with_etag(request, etag)),
                Err(cache_guard) => Some(cache_guard),
            }
        } else {
            None
        };

        let url_prefix = request.get_param("url_prefix").as_deref().unwrap_or("www.gov.uk/").parse::<HttpsStrippedUrl>().map_err(|_| Error::InvalidRequest)?.0;
        let tag = request.get_param("tag").filter(|t| !t.is_empty()).map(Tag::new);

        let updates = data.list_updates(&url_prefix, tag);

        let (html, etag) = updates_page_response(updates,request,data);
        if let Some(mut cache_guard) = cache_guard {
            *cache_guard = Some((data_updated_at, Arc::new((html.clone(), etag.clone()))));
            drop(cache_guard)
        }
        Ok(Response::html(html).with_etag(request, etag))
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
            tags = data.get_tags(update.update_ref()).iter().map(|u| u.name()).collect::<String>(),
            diff_url = diff_url,
            doc_from = from_ts.map_or(String::new(), |v| v.to_string()),
            doc_to = to_ts.map_or(String::new(), |v| v.to_string()),
            body = body,
            history = updates.iter().rev().fold(String::new(), |mut acc, (_, (update, _tags))| {
                write!(
                    acc,
                    r#"<a href="/update/{}/{}{}"><p class="update-description">{}<br />{}</p></a>"#,
                    update.timestamp().to_rfc3339(),
                    update.url().host_str().unwrap(),
                    update.url().path(),
                    update.timestamp().format("%F %H:%M"),
                    update.change()
                )
                .unwrap();
                acc
            }),
        ))
        .with_status_code(if from_ts.is_none() && to_ts.is_none() { 404 } else { 200 })
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
        .with_status_code(if from_ts.is_none() && to_ts.is_none() { 404 } else { 200 })
        .with_etag(request, format!("{} {}", from_doc.is_some(), to_doc.is_some())))
    }
}

fn updates_page_response<'a>(
    updates: impl Iterator<Item = &'a Update>,
    request: &Request,
    data: &Data,
) -> (String, String) {
    let mut results = UpdateList::new(updates, request, data);
    let etag = results.etag();
    let mut result_string = String::new(); // ugh
    results.render_into(&mut result_string).unwrap();
    let selected_tag = request.get_param("tag");
    let html = format!(
        include_str!("updates.html"),
        result_string,
        url_prefix_filter = request.get_param("url_prefix").as_deref().unwrap_or("www.gov.uk/"),
        change_filter = request.get_param("change").as_deref().unwrap_or(""),
        tag_options = data.all_tags().fold(String::new(), |mut acc, tag| {
            write!(
                acc,
                r#"<option {selected}>{tag}</option>"#,
                tag = tag,
                selected = (selected_tag.as_ref() == Some(tag))
                    .then_some("selected")
                    .unwrap_or_default()
            )
            .unwrap();
            acc
        }),
    );
    (html, etag)
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

    (
        format!("{}{}", diff_base, url.path()),
        from.map(DocumentVersion::timestamp).copied(),
        to.map(DocumentVersion::timestamp).copied(),
        match (from, to) {
            (Some(from), Some(to)) => {
                let cache = env::var("DIFFCACHE").ok();
                let cached_diff = if let Some(cache) = &cache.as_deref() {
                    match cacache::read_sync(cache, &diff_base) {
                        Ok(from_cache) => String::from_utf8(from_cache).ok(),
                        Err(cacache::Error::EntryNotFound(_, _)) => None,
                        Err(err) => {
                            println!("Error reading from cache : {:?}", err);
                            if let Err(err) = cacache::remove_sync(cache, &diff_base) {
                                println!("Error removing from cache : {:?}", err);
                            }
                            None
                        }
                    }
                } else {
                    None
                };
                cached_diff.unwrap_or_else(|| {
                    let diff = data
                        .read_doc_to_string(from)
                        .with_base_url(&diff_base)
                        .diff(&data.read_doc_to_string(to).with_base_url(&diff_base));
                    if let Some(cache) = &cache {
                        if let Err(err) = cacache::write_sync(cache, &diff_base, &diff) {
                            println!("Error writing to cache : {:?}", err);
                        }
                    }
                    diff
                })
            }
            (Some(from), None) => data.read_doc_to_string(from).with_base_url(&diff_base).into_inner(),
            (None, Some(to)) => data.read_doc_to_string(to).with_base_url(&diff_base).into_inner(),
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

    fn render_into(mut self, f: &mut String) -> fmt::Result {
        let mut current_date = None;
        writeln!(
            f,
            r#"
    <div class="commit-log">
        <div class="table-header">Filename on gov.uk</div>
        <div class="table-header">Change description</div>
        <div class="table-header">Tags</div>"#
        )?;

        for update in &mut self.page {
            let update_date = update.timestamp().date_naive();
            if Some(update_date) != current_date {
                current_date = Some(update_date);
                writeln!(f, r#"<h3 class="date-seperator">{}</h3>"#, update_date).unwrap();
            }
            let updated_doc_path = update.url().path();
            let update_path = format!(
                "/update/{}/{}{updated_doc_path}",
                update.timestamp().to_rfc3339(),
                update.url().host_str().unwrap_or_default(),
            );
            write!(
                f,
                r#"<a href="{update_path}" class="update-url">{updated_doc_path}</a>
                <a href="{update_path}" class="update-description">{change_time} {change_description}</a>
                <a href="{update_path}" class="update-tags">
                "#,
                change_time = update.timestamp().time().format_with_items(StrftimeItems::new("%H:%M")),
                change_description = update.change(),
            )?;
            for tag in self.data.get_tags(update.update_ref()) {
                writeln!(f, "<div>{}</div>", tag.name())?;
            }
            writeln!(f, r#"</a>"#)?;
        }

        writeln!(f, "</div>")?;
        self.page.render_pagination_into(f)?;
        writeln!(f, "</div>")?;
        Ok(())
    }
}

impl<'a, Us: Iterator<Item = &'a Update>> UpdateList<'a, '_, Us> {
    fn etag(&mut self) -> String {
        mem::take(&mut self.etag)
    }
}

/// An shared in memory cache for a single page and it's etag. If the cache is invalidated, the first caller will get access to the write guard to update it, the rest will wait
#[derive(Debug, Default)]
struct FastCache(Arc<RwLock<FastCacheInternal>>);
type FastCacheInternal = Option<(Instant, Arc<(String, String)>)>;

impl FastCache {
    fn try_cache(&self, oldest_allowed: Instant) -> Result<(String, String), RwLockWriteGuard<FastCacheInternal>> {
        if let Ok(guard) = self.0.read() {
            if let Some((rendered_at, cached)) = &*guard {
                if oldest_allowed <= *rendered_at {
                    // cached page is still valid
                    let cached = cached.clone();
                    drop(guard);
                    return Ok(cached.deref().clone());
                }
            }
        }
        // cache invalid, empty or poisoned, promote to write lock
        match self.0.write() {
            Ok(guard) => {
                // check if another thread already freshened the cache enough
                if let Some((rendered_at, cached)) = &*guard {
                    if oldest_allowed < *rendered_at {
                        // cached page is still valid
                        let cached = cached.clone();
                        drop(guard);
                        Ok(cached.deref().clone())
                    } else {
                        Err(guard)
                    }
                } else {
                    Err(guard)
                }
            }
            Err(poisoned) => Err(poisoned.into_inner()),
        }
    }
}
