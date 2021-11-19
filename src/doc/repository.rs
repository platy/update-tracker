use super::*;
use crate::{
    repository::WriteResult,
    url::{IterUrlRepoLeaves, UrlRepo},
};

use chrono::DateTime;
use std::{
    error::Error,
    fs,
    io,
    path::{Path, PathBuf},
};

pub struct DocRepo {
    repo: UrlRepo,
}

impl DocRepo {
    pub fn new(base: impl AsRef<Path>) -> io::Result<Self> {
        let repo = UrlRepo::new("docver", base)?;
        Ok(Self { repo })
    }

    /// Create a [`DocumentVersion`] and return a writer to write the content
    pub fn create(&self, url: Url, timestamp: DateTime<FixedOffset>) -> io::Result<TempDoc> {
        let doc = DocumentVersion { url, timestamp };
        let path = self.path_for_version(&doc);
        let is_new_doc = !self.document_exists(&doc.url)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        let open_neighbour = |dv| -> io::Result<_> {
            let path = self.path_for_version(&dv);
            let file = fs::File::open(&path)?;
            Ok((dv, file))
        };
        let (before, after) = self
            .neighbours(&doc)
            .map_err(|e| NeighbourCheckError::io(e, &"Finding neighbours"))?;
        let identical_before = before.map(open_neighbour).transpose()?;
        let identical_after = after.map(open_neighbour).transpose()?;
        Ok(TempDoc {
            is_new_doc,
            doc,
            file,
            repo: self,
            identical_before,
            identical_after,
            buffer: [0; DUPLICATE_CHECK_BUFFER_SIZE],
        })
    }

    /// Open a [`DocumentVersion`] for reading
    pub fn open(&self, doc_version: &DocumentVersion) -> io::Result<impl io::Read> {
        fs::File::open(self.path_for_version(doc_version))
    }

    /// Ensure that a [`DocumentVersion`] exists for a given url and timestamp
    pub fn ensure_version(&self, url: Url, timestamp: DateTime<FixedOffset>) -> io::Result<DocumentVersion> {
        let doc_version = DocumentVersion { url, timestamp };
        fs::File::open(self.path_for_version(&doc_version))?;
        Ok(doc_version)
    }

    /// Find chronological neighbours of this DocumentVersion
    fn neighbours(
        &self,
        DocumentVersion {
            url: r_url,
            timestamp: r_ts,
        }: &DocumentVersion,
    ) -> io::Result<(Option<DocumentVersion>, Option<DocumentVersion>)> {
        let mut before: Option<DocumentVersion> = None;
        let mut after: Option<DocumentVersion> = None;
        for result in self.list_versions(r_url.to_owned())? {
            let candidate = result?;
            match candidate.timestamp.cmp(r_ts) {
                std::cmp::Ordering::Less => {
                    if let Some(before) = &mut before {
                        if candidate.timestamp > before.timestamp {
                            *before = candidate;
                        }
                    } else {
                        before = Some(candidate);
                    }
                }
                std::cmp::Ordering::Greater => {
                    if let Some(after) = &mut after {
                        if candidate.timestamp < after.timestamp {
                            *after = candidate;
                        }
                    } else {
                        after = Some(candidate);
                    }
                }
                std::cmp::Ordering::Equal => {}
            }
        }
        Ok((before, after))
    }

    /// Lists all updates on the specified url from newest to oldest
    pub fn list_versions(&self, url: Url) -> io::Result<impl Iterator<Item = io::Result<DocumentVersion>>> {
        let files = self.repo.read_leaves_sorted_for_url(&url)?;

        Ok(files.rev().map(move |dir_entry| {
            let timestamp = dir_entry
                .0
                .parse()
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
            Ok(DocumentVersion {
                url: url.clone(),
                timestamp,
            })
        }))
    }

    /// Lists all updates
    pub fn list_all(&self, base_url: &Url) -> io::Result<IterUrlRepoLeaves<'_, DocumentVersion>> {
        self.repo.list_all(base_url.clone(), |url, name, _| {
            let timestamp = name
                .parse()
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
                .unwrap();
            DocumentVersion { url, timestamp }
        })
    }

    pub fn document_exists(&self, url: &Url) -> io::Result<bool> {
        match self.repo.read_leaves_for_url(url) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Ok(mut iter) => Ok(iter.next().is_some()),
            Err(err) => Err(err),
        }
    }

    fn path_for_version(&self, DocumentVersion { url, timestamp }: &DocumentVersion) -> PathBuf {
        self.repo.leaf_path(url, &timestamp.to_rfc3339())
    }
}

const DUPLICATE_CHECK_BUFFER_SIZE: usize = 1024;

/// TODO Maybe this should write to a temp file to start with and then be moved into place, that way the whole repo structure will consist of complete files
pub struct TempDoc<'r> {
    is_new_doc: bool, // TODO replace with something better when fixing the above
    doc: DocumentVersion,
    file: fs::File,
    repo: &'r DocRepo,
    /// if `Some` this is a version that is timestamped directly before the one being written, as as far as the current doc has been written, both are identical
    identical_before: Option<(DocumentVersion, fs::File)>,
    /// like `identical_before` but with a version timestamped directly after the one being written
    identical_after: Option<(DocumentVersion, fs::File)>,
    buffer: [u8; DUPLICATE_CHECK_BUFFER_SIZE],
}

impl TempDoc<'_> {
    pub fn done(mut self) -> WriteResult<DocumentVersion, 2> {
        use io::Write;

        self.file.flush()?;
        // TODO check that any neighbour files have reached EOF, ohterwise set them to none
        if let Some((before, _)) = self.identical_before {
            fs::remove_file(self.repo.path_for_version(&self.doc))?;
            before.with_events([None, None])
        } else if let Some((after, _)) = self.identical_after {
            fs::remove_file(self.repo.path_for_version(&after))?;
            let events = [Some(DocEvent::updated(&self.doc)), Some(DocEvent::deleted(&after))];
            self.doc.with_events(events)
        } else {
            let events = [
                Some(DocEvent::updated(&self.doc)),
                self.is_new_doc.then(|| DocEvent::created(&self.doc)),
            ];
            self.doc.with_events(events)
        }
    }

    fn check_duplicate_neighbours(&mut self, buf: &[u8]) -> io::Result<()> {
        use io::Read;

        let mut comparison_buf = &mut self.buffer[..buf.len()];
        if let Some((_, file)) = &mut self.identical_before {
            match file.read_exact(&mut comparison_buf) {
                Err(e) => {
                    if e.kind() == io::ErrorKind::UnexpectedEof {
                        self.identical_before = None;
                    } else {
                        return Err(NeighbourCheckError::io(
                            e,
                            &"Reading earlier neighbour to check for identity",
                        ));
                    }
                }
                Ok(()) => {
                    if comparison_buf != buf {
                        self.identical_before = None;
                    }
                }
            }
        }
        if let Some((_, file)) = &mut self.identical_after {
            match file.read_exact(&mut comparison_buf) {
                Err(e) => {
                    if e.kind() == io::ErrorKind::UnexpectedEof {
                        self.identical_after = None;
                    } else {
                        return Err(NeighbourCheckError::io(
                            e,
                            &"Reading later neighbour to check for identity",
                        ));
                    }
                }
                Ok(()) => {
                    if comparison_buf != buf {
                        self.identical_after = None;
                    }
                }
            }
        }
        Ok(())
    }
}

impl io::Write for TempDoc<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.file.write(buf)?;
        for check in buf[0..written].chunks(DUPLICATE_CHECK_BUFFER_SIZE) {
            self.check_duplicate_neighbours(check)?;
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

struct NeighbourCheckError {
    source: io::Error,
    description: &'static &'static str,
}

impl NeighbourCheckError {
    fn io(e: io::Error, arg: &'static &'static str) -> io::Error {
        io::Error::new(
            io::ErrorKind::Other,
            Self {
                source: e,
                description: arg,
            },
        )
    }
}

impl Error for NeighbourCheckError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

impl fmt::Display for NeighbourCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "'{}' caused by:", self.description)?;
        self.source.fmt(f)
    }
}

impl fmt::Debug for NeighbourCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "'{}' caused by:", self.description)?;
        self.source.fmt(f)
    }
}

#[cfg(test)]
mod test {
    use std::io::{Read, Write};

    use chrono::Utc;

    use super::*;

    #[test]
    fn new_doc_creates_events_and_becomes_available() {
        let repo = test_repo("new_doc_creates_events_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let doc_content = "test document";
        let timestamp = Utc::now().into();
        let should = DocumentVersion {
            url: url.clone(),
            timestamp,
        };
        let mut buf = vec![];

        let mut write = repo.create(url.clone(), timestamp).unwrap();
        write.write_all(doc_content.as_bytes()).unwrap();

        let doc = write.done().unwrap();
        assert_eq!(*doc, should);
        repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, doc_content.as_bytes());

        assert_eq!(
            doc.into_events().collect::<Vec<_>>(),
            [
                DocEvent::Updated {
                    url: url.clone(),
                    timestamp
                },
                DocEvent::Created { url: url.clone() }
            ]
        );

        let doc: DocumentVersion = repo.ensure_version(url.clone(), timestamp).unwrap();
        assert_eq!(doc, should);
        buf.clear();
        repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, doc_content.as_bytes());

        let doc = repo.list_versions(url.clone()).unwrap().next().unwrap().unwrap();
        assert_eq!(doc, should);
        buf.clear();
        repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, doc_content.as_bytes());
    }

    #[test]
    fn updated_doc_creates_event_and_becomes_available() {
        let repo = test_repo("updated_doc_creates_event_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let doc_content = "new content";
        let timestamp = Utc::now().into();
        let should = DocumentVersion {
            url: url.clone(),
            timestamp,
        };
        let mut buf = vec![];

        let mut write = repo
            .create(
                url.clone(),
                DateTime::<FixedOffset>::from(Utc::now()) - chrono::Duration::seconds(60),
            )
            .unwrap();
        write.write_all("old content".as_bytes()).unwrap();
        let doc = write.done().unwrap();
        repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();

        let mut write = repo.create(url.clone(), timestamp).unwrap();
        write.write_all(doc_content.as_bytes()).unwrap();
        let doc = write.done().unwrap();
        assert_eq!(*doc, should);
        buf.clear();
        repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, doc_content.as_bytes());

        assert_eq!(
            doc.into_events().collect::<Vec<_>>(),
            [DocEvent::Updated {
                url: url.clone(),
                timestamp
            },]
        );

        let doc: DocumentVersion = repo.ensure_version(url.clone(), timestamp).unwrap();
        assert_eq!(doc, should);
        buf.clear();
        repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, doc_content.as_bytes());

        let mut list = repo.list_versions(url.clone()).unwrap();
        let doc = list.next().unwrap().unwrap();
        assert_eq!(doc, should);
        buf.clear();
        repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, doc_content.as_bytes());
        let _old = list.next().unwrap().unwrap();
    }

    #[test]
    fn new_duplicate_is_deduplicated() {
        let repo = test_repo("new_duplicate_is_deduplicated");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let doc_content = "content";
        let earlier_timestamp = (Utc::now() - chrono::Duration::seconds(60)).into();
        let later_timestamp = Utc::now().into();
        let should = DocumentVersion {
            url: url.clone(),
            timestamp: earlier_timestamp,
        };

        let mut write = repo.create(url.clone(), earlier_timestamp).unwrap();
        write.write_all(doc_content.as_bytes()).unwrap();
        let doc = write.done().unwrap();
        assert_eq!(*doc, should);

        let mut write = repo.create(url.clone(), later_timestamp).unwrap();
        write.write_all("content".as_bytes()).unwrap();
        let doc2 = write.done().unwrap();
        assert_eq!(*doc, *doc2);

        assert_eq!(doc2.into_events().count(), 0);
    }

    #[test]
    fn old_duplicate_is_deduplicated() {
        let repo = test_repo("old_duplicate_is_deduplicated");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let doc_content = "content";
        let earlier_timestamp = (Utc::now() - chrono::Duration::seconds(60)).into();
        let later_timestamp = Utc::now().into();
        let should = DocumentVersion {
            url: url.clone(),
            timestamp: earlier_timestamp,
        };

        let mut write = repo.create(url.clone(), later_timestamp).unwrap();
        write.write_all("content".as_bytes()).unwrap();
        let doc = write.done().unwrap();

        let mut write = repo.create(url.clone(), earlier_timestamp).unwrap();
        write.write_all(doc_content.as_bytes()).unwrap();
        let doc2 = write.done().unwrap();
        assert_eq!(*doc2, should);

        assert!(repo.open(&doc).is_err());

        assert_eq!(
            doc2.into_events().collect::<Vec<_>>(),
            [
                DocEvent::Updated {
                    url: url.clone(),
                    timestamp: earlier_timestamp
                },
                DocEvent::Deleted {
                    url: url.clone(),
                    timestamp: later_timestamp
                }
            ]
        );
    }

    #[test]
    fn list_versions() {
        let repo = test_repo("doc::list_versions");

        let docs = &[
            ("http://www.example.org/test/doc1", "2021-03-01T10:00:00+00:00", "1"),
            ("http://www.example.org/test/doc1", "2021-03-01T11:00:00+00:00", "2"),
            ("http://www.example.org/test/doc1", "2021-03-01T12:00:00+00:00", "3"),
            ("http://www.example.org/test/doc2", "2021-03-01T11:00:00+00:00", "4"),
            ("http://www.example.org/test/doc2", "2021-03-01T12:00:00+00:00", "5"),
        ];

        for (url, timestamp, content) in docs {
            let mut write = repo.create(url.parse().unwrap(), timestamp.parse().unwrap()).unwrap();
            write.write_all(content.as_bytes()).unwrap();
            let _ = write.done().unwrap();
        }

        let mut buf = Vec::new();
        let result = repo
            .list_versions("http://www.example.org/test/doc1".parse().unwrap())
            .unwrap();
        for (&(e_url, e_ts, e_content), actual) in docs[0..3].iter().rev().zip(result) {
            let actual = actual.unwrap();
            assert_eq!(actual.url().as_str(), e_url);
            assert_eq!(actual.timestamp.to_rfc3339(), e_ts);
            buf.clear();
            repo.open(&actual).unwrap().read_to_end(&mut buf).unwrap();
            assert_eq!(buf, e_content.as_bytes());
        }
    }

    #[test]
    fn list_all() {
        let repo = test_repo("doc::list_all");

        let docs = &[
            ("http://www.example.org/test/doc1", "2021-03-01T10:00:00+00:00", "1"),
            ("http://www.example.org/test/doc1", "2021-03-01T11:00:00+00:00", "2"),
            ("http://www.example.org/test/doc1", "2021-03-01T12:00:00+00:00", "3"),
            ("http://www.example.org/test/doc2", "2021-03-01T11:00:00+00:00", "4"),
            ("http://www.example.org/test/doc2", "2021-03-01T12:00:00+00:00", "5"),
        ];

        for (url, timestamp, content) in docs {
            let mut write = repo.create(url.parse().unwrap(), timestamp.parse().unwrap()).unwrap();
            write.write_all(content.as_bytes()).unwrap();
            let _ = write.done().unwrap();
        }

        let mut buf = Vec::new();
        let result: Vec<_> = repo
            .list_all(&"http://www.example.org/".parse().unwrap())
            .unwrap()
            .map(|r| {
                let doc = r.unwrap();
                buf.clear();
                repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();
                (
                    doc.url().to_string(),
                    doc.timestamp.to_rfc3339(),
                    String::from_utf8(buf.clone()).unwrap(),
                )
            })
            .collect();
        let sliced: Vec<_> = result.iter().map(|(a, b, c)| (&a[..], &b[..], &c[..])).collect();
        assert_eq!(sliced, docs);
    }

    fn test_repo(name: &str) -> DocRepo {
        let path = format!("tmp/{}", name);
        let _ = fs::remove_dir_all(&path);
        let repo = DocRepo::new(path).unwrap();
        repo
    }
}
