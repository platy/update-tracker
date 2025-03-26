use std::fmt;

use askama::Template;
use rouille::Request;

pub struct Page<I> {
    href: String,
    offset: usize,
    limit: usize,
    emitted: usize,
    items: std::iter::Skip<I>,
}

impl<T, I: Iterator<Item = T>> Page<I> {
    pub fn new(request: &Request, items: I) -> Self {
        let offset = request
            .get_param("offset")
            .and_then(|offset| offset.parse().ok())
            .unwrap_or(0);
        let limit = request
            .get_param("limit")
            .and_then(|limit| limit.parse().ok())
            .unwrap_or(200);

        let existing_pairs = request.raw_query_string().to_owned();
        let mut href = form_urlencoded::Serializer::new(request.url() + "?");
        for (name, value) in form_urlencoded::parse(existing_pairs.as_bytes()) {
            if name != "offset" {
                href.append_pair(&name, &value);
            }
        }
        let href = href.finish();

        let items = items.skip(offset);

        Self {
            href,
            offset,
            limit,
            items,
            emitted: 0,
        }
    }

    /// takes ownership of the page in order to count the remaining items following this page from the iterator
    pub fn render_pagination_into(self, f: &mut String) -> fmt::Result {
        let offset = self.offset;
        let limit = self.limit;

        let filtered_count = offset + self.emitted + self.items.count();

        PaginationTemplate {
            href: self.href,
            offset,
            emitted: self.emitted,
            filtered_count,
            page_num: offset / limit + 1,
            page_count: filtered_count / limit + 1,
            prev_offset: (offset > 0).then(|| offset.checked_sub(limit).unwrap_or_default()),
            next_offset: (offset + limit <= filtered_count).then(|| offset + limit),
        }
        .render_into(f)
        .unwrap();
        Ok(())
    }
}

#[derive(Template)]
#[template(path = "page.html")]
pub struct PaginationTemplate {
    href: String,
    offset: usize,
    emitted: usize,
    filtered_count: usize,
    page_num: usize,
    page_count: usize,
    prev_offset: Option<usize>,
    next_offset: Option<usize>,
}

impl<T, I: Iterator<Item = T>> Iterator for Page<I> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.emitted >= self.limit {
            return None;
        }
        let r = self.items.next();
        if r.is_some() {
            self.emitted += 1;
        }
        r
    }
}
