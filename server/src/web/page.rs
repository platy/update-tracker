use std::fmt::{self, Write};

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

    pub fn into_writer(self, f: &mut String) -> fmt::Result {
        let offset = self.offset;
        let limit = self.limit;

        let filtered_count = offset + self.emitted + self.items.count();

        let page_num = offset / limit + 1;
        let page_count = filtered_count / limit + 1;

        let prev_offset = (offset > 0).then(|| offset.checked_sub(limit).unwrap_or_default());
        let next_offset = (offset + limit <= filtered_count).then(|| offset + limit);

        if let Some(prev_offset) = prev_offset {
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
            page_num = page_num,
            page_count = page_count,
            offset = offset + 1,
            last = offset + self.emitted,
            total = filtered_count,
        )?;
        if let Some(next_offset) = next_offset {
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
