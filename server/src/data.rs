use std::{
    collections::{HashMap, HashSet},
    io::{self, Read},
    ops::Deref,
    sync::Arc,
};

use chrono::{DateTime, FixedOffset};
use htmldiff::htmldiff;
use tantivy::{
    collector::TopDocs,
    doc,
    query::{AllQuery, QueryParser},
    DocAddress, Document, Index,
};
use update_repo::{
    doc::{DocRepo, DocumentVersion},
    tag::TagRepo,
    update::UpdateRepo,
    Url,
};

pub(crate) struct Data {
    doc_repo: DocRepo,
    all_tags: Vec<String>,
    pub text_index: Index,
    pub query_parser: QueryParser,
    url_field: tantivy::schema::Field,
    timestamp_field: tantivy::schema::Field,
    tz_offset_field: tantivy::schema::Field,
    change_field: tantivy::schema::Field,
    tag_field: tantivy::schema::Field,
}

impl Data {
    pub fn new() -> Self {
        let update_repo = UpdateRepo::new("../repo/url").unwrap();
        let doc_repo = DocRepo::new("../repo/url").unwrap();

        // full text schema
        use tantivy::schema::*;
        let mut schema_builder = Schema::builder();
        let url_field = schema_builder.add_facet_field("url", INDEXED | STORED);
        let timestamp_field = schema_builder.add_date_field("timestamp", STORED | FAST);
        let tz_offset_field = schema_builder.add_i64_field("tz_offset", STORED);
        let change_field = schema_builder.add_text_field("change", TEXT | STORED);
        let tag_field = schema_builder.add_text_field("tag", TEXT | STORED);
        let schema = schema_builder.build();

        // full text index
        std::fs::remove_dir_all("../repo/text").unwrap();
        std::fs::create_dir("../repo/text").unwrap();
        let text_index = Index::create_in_dir("../repo/text", schema).unwrap();
        // let text_index = Index::open_in_dir("../repo/text").unwrap();
        let mut index_writer = text_index.writer(50_000_000).unwrap();

        let mut tags: HashMap<_, HashSet<_>> = HashMap::new();

        // add taggings to the indices
        let tag_repo = TagRepo::new("../repo/tag").unwrap();
        let mut all_tags = vec![];
        for tag in tag_repo.list_tags().unwrap() {
            println!("Tag {}", tag.name());
            all_tags.push(tag.name().to_owned());
            let tag = Arc::new(tag);
            for ur in tag_repo.list_updates_in_tag(&tag).unwrap() {
                let ur = ur.unwrap();
                tags.entry(ur).or_default().insert(tag.clone());
            }
        }

        // add updates to the indices
        for update in update_repo.list_all(&"https://www.gov.uk/".parse().unwrap()).unwrap() {
            let update = update.unwrap();
            let url = Facet::from_text(update.url().as_str().strip_prefix("https:/").unwrap()).unwrap();
            let mut document = doc!(
                url_field => url,
                timestamp_field => update.timestamp().with_timezone(&chrono::Utc),
                tz_offset_field => update.timestamp().offset().local_minus_utc() as i64,
                change_field => update.change(),
            );
            for tag in tags.get(update.update_ref()).iter().flat_map(|s| s.iter()) {
                document.add_text(tag_field, tag);
            }
            index_writer.add_document(document);
        }

        index_writer.commit().unwrap();

        // full text parser
        let query_parser = QueryParser::for_index(&text_index, vec![change_field, tag_field]);

        Self {
            doc_repo,
            all_tags,
            text_index,
            query_parser,
            url_field,
            timestamp_field,
            tz_offset_field,
            change_field,
            tag_field,
        }
    }

    pub fn search(&self, q: Option<&str>) -> impl Iterator<Item = Update> + '_ {
        let reader = self.text_index.reader().unwrap();
        let searcher = reader.searcher();

        let query = if let Some(q) = q {
            self.query_parser.parse_query(q).unwrap()
        } else {
            Box::new(AllQuery)
        };

        // Perform search.
        let top_docs: Vec<(_, DocAddress)> = searcher
            .search(
                &query,
                &TopDocs::with_limit(10).order_by_u64_field(self.timestamp_field),
            )
            .unwrap();

        top_docs.into_iter().map(move |(_, doc)| {
            let doc = searcher.doc(doc).unwrap();
            Update(self, doc)
        })
    }

    pub(crate) fn get_doc_version(&self, url: &Url, timestamp: DateTime<FixedOffset>) -> io::Result<DocumentVersion> {
        self.doc_repo.ensure_version(url.to_owned(), timestamp)
    }

    pub fn read_doc_to_string(&self, doc: &DocumentVersion) -> DocBody {
        let mut body = String::new();
        self.doc_repo.open(doc).unwrap().read_to_string(&mut body).unwrap();
        DocBody(body)
    }

    pub fn all_tags(&self) -> impl Iterator<Item = &String> {
        self.all_tags.iter()
    }
}

pub struct Update<'a>(&'a Data, Document);

impl<'a> Update<'a> {
    pub fn url(&self) -> Url {
        format!("https:/{}", self.1.get_first(self.0.url_field).unwrap().path().unwrap())
            .parse::<url::Url>()
            .unwrap()
            .into()
    }

    pub fn timestamp(&self) -> DateTime<FixedOffset> {
        let timestamp = self.1.get_first(self.0.timestamp_field).unwrap().date_value().unwrap();
        let secs_east = self.1.get_first(self.0.tz_offset_field).unwrap().i64_value().unwrap();
        let offset = FixedOffset::east(secs_east as i32);
        timestamp.with_timezone(&offset)
    }

    pub fn change(&self) -> &str {
        self.1.get_first(self.0.change_field).unwrap().text().unwrap()
    }

    pub fn tags(&self) -> impl Iterator<Item = &str> {
        self.1.get_all(self.0.tag_field).map(|v| v.text().unwrap())
    }
}

pub struct DocBody(String);

impl DocBody {
    pub fn diff(&self, other: &Self) -> String {
        htmldiff(&self.0, &other.0)
    }

    pub fn with_base_url(self, base_url: &str) -> Self {
        let replace = format!("href=\"{}/", base_url);
        DocBody(self.0.replace("href=\"/", &replace))
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl Deref for DocBody {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
