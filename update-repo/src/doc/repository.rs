use super::*;
use crate::{
    repository::WriteResult,
    url::{IterUrlRepoLeaves, UrlRepo},
};

use chrono::DateTime;
use core::panic;
use std::{
    error::Error,
    fs,
    io::{self, Read},
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
    pub fn create<'r>(
        &'r self,
        url: Url,
        timestamp: DateTime<FixedOffset>,
        write_avoidance_buffer: &'r mut Vec<u8>,
    ) -> io::Result<DeduplicatingWriter<'r>> {
        let doc = DocumentVersion { url, timestamp };
        write_avoidance_buffer.clear();
        DeduplicatingWriter::new(doc, self, write_avoidance_buffer)
    }

    /// Open a [`DocumentVersion`] for reading
    pub fn open(&self, doc_version: &DocumentVersion) -> io::Result<impl io::Read + io::Seek> {
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
const WRITE_AVOIDANCE_BUFFER_LIMIT: usize = 32 * 1024;

/// TODO Maybe this should write to a temp file to start with and then be moved into place, that way the whole repo structure will consist of complete files
pub struct DeduplicatingWriter<'r> {
    doc: DocumentVersion,
    state: DeduplicatingWriterState<'r>,
    repo: &'r DocRepo,
    /// if `Some` this is a version that is timestamped directly before the one being written, as as far as the current doc has been written, both are identical
    identical_before: Option<(DocumentVersion, fs::File)>,
    /// like `identical_before` but with a version timestamped directly after the one being written
    identical_after: Option<(DocumentVersion, fs::File)>,
    buffer: [u8; DUPLICATE_CHECK_BUFFER_SIZE],
}
enum DeduplicatingWriterState<'b> {
    /// the file is being directly written to
    Writing {
        file: fs::File,
        is_new_doc: bool, // TODO replace with something better when fixing the above
    },
    /// writing to a buffer to optimise for the case that it is a duplicate and doesn't need to be written
    Buffering(io::Cursor<&'b mut Vec<u8>>),
}
impl<'r> DeduplicatingWriter<'r> {
    fn new(doc: DocumentVersion, repo: &'r DocRepo, write_avoidance_buffer: &'r mut Vec<u8>) -> io::Result<Self> {
        let path = repo.path_for_version(&doc);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let open_neighbour = |dv| -> io::Result<_> {
            let path = repo.path_for_version(&dv);
            let file = fs::File::open(&path)?;
            Ok((dv, file))
        };
        let (before, after) = repo
            .neighbours(&doc)
            .map_err(|e| NeighbourCheckError::io(e, &"Finding neighbours"))?;
        let identical_before = before.map(open_neighbour).transpose()?;
        let identical_after = after.map(open_neighbour).transpose()?;
        Ok(Self {
            doc,
            state: if identical_before.is_none() && identical_after.is_none() {
                DeduplicatingWriterState::Writing {
                    file: fs::OpenOptions::new().write(true).create_new(true).open(&path)?,
                    is_new_doc: true,
                }
            } else {
                DeduplicatingWriterState::Buffering(io::Cursor::new(write_avoidance_buffer))
            },
            repo,
            identical_before,
            identical_after,
            buffer: [0; DUPLICATE_CHECK_BUFFER_SIZE],
        })
    }

    fn really_flush(&mut self) -> io::Result<(bool, &mut fs::File)> {
        use io::Write;

        Ok(match self.state {
            DeduplicatingWriterState::Writing {
                ref mut file,
                is_new_doc,
            } => {
                file.flush()?;
                (is_new_doc, file)
            }
            DeduplicatingWriterState::Buffering(ref buffer) => {
                let path = self.repo.path_for_version(&self.doc);
                let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
                file.write_all(buffer.get_ref())?;
                file.flush()?;
                self.state = DeduplicatingWriterState::Writing {
                    file,
                    is_new_doc: false,
                };
                if let DeduplicatingWriterState::Writing { file, is_new_doc: _ } = &mut self.state {
                    (false, file)
                } else {
                    panic!();
                }
            }
        })
    }

    pub fn done(mut self) -> WriteResult<DocumentVersion, 2> {
        if let Some((_, file)) = &mut self.identical_before {
            if file.read(&mut [0]).is_err() {
                // file is EOF, so finishes at this point too
                self.identical_before = None;
            }
        }
        if let Some((_, file)) = &mut self.identical_after {
            if file.read(&mut [0]).is_err() {
                // file is EOF, so finishes at this point too
                self.identical_after = None;
            }
        }
        if let Some((before, _)) = self.identical_before {
            if let DeduplicatingWriterState::Writing { .. } = self.state {
                fs::remove_file(self.repo.path_for_version(&self.doc))?;
            }
            return before.with_events([None, None]);
        }
        let (is_new_doc, _file) = self.really_flush()?;
        if let Some((after, _)) = self.identical_after {
            fs::remove_file(self.repo.path_for_version(&after))?;
            let events = [Some(DocEvent::updated(&self.doc)), Some(DocEvent::deleted(&after))];
            return self.doc.with_events(events);
        }
        let events = [
            Some(DocEvent::updated(&self.doc)),
            is_new_doc.then(|| DocEvent::created(&self.doc)),
        ];
        self.doc.with_events(events)
    }

    fn check_duplicate_neighbours(&mut self, buf: &[u8]) -> io::Result<()> {
        let comparison_buf = &mut self.buffer[..buf.len()];
        if let Some((_, file)) = &mut self.identical_before {
            match file.read_exact(comparison_buf) {
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
            match file.read_exact(comparison_buf) {
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

impl io::Write for DeduplicatingWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = match &mut self.state {
            DeduplicatingWriterState::Writing { is_new_doc: _, file } => file.write(buf)?,
            DeduplicatingWriterState::Buffering(write_avoidance_buffer) => {
                if write_avoidance_buffer.get_ref().len() + buf.len() > WRITE_AVOIDANCE_BUFFER_LIMIT {
                    let (_is_new_doc, file) = self.really_flush()?;
                    file.write(buf)?
                } else {
                    write_avoidance_buffer.write(buf)?
                }
            }
        };
        for check in buf[0..written].chunks(DUPLICATE_CHECK_BUFFER_SIZE) {
            self.check_duplicate_neighbours(check)?;
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        panic!();
        // self.file.flush()
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

        let mut write_avoidance_buffer = Vec::new();
        let mut write = repo
            .create(url.clone(), timestamp, &mut write_avoidance_buffer)
            .unwrap();
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

        let mut write_avoidance_buffer = Vec::new();
        let mut write = repo
            .create(
                url.clone(),
                DateTime::<FixedOffset>::from(Utc::now()) - chrono::Duration::seconds(60),
                &mut write_avoidance_buffer,
            )
            .unwrap();
        write.write_all("old content".as_bytes()).unwrap();
        let doc = write.done().unwrap();
        repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();

        let mut write = repo
            .create(url.clone(), timestamp, &mut write_avoidance_buffer)
            .unwrap();
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

        let mut write_avoidance_buffer = Vec::new();
        let mut write = repo
            .create(url.clone(), earlier_timestamp, &mut write_avoidance_buffer)
            .unwrap();
        write.write_all(doc_content.as_bytes()).unwrap();
        let doc = write.done().unwrap();
        assert_eq!(*doc, should);

        let mut write = repo
            .create(url.clone(), later_timestamp, &mut write_avoidance_buffer)
            .unwrap();
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

        let mut write_avoidance_buffer = Vec::new();
        let mut write = repo
            .create(url.clone(), later_timestamp, &mut write_avoidance_buffer)
            .unwrap();
        write.write_all("content".as_bytes()).unwrap();
        let doc = write.done().unwrap();

        let mut write = repo
            .create(url.clone(), earlier_timestamp, &mut write_avoidance_buffer)
            .unwrap();
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

        let mut write_avoidance_buffer = Vec::new();
        for (url, timestamp, content) in docs {
            let mut write = repo
                .create(
                    url.parse().unwrap(),
                    timestamp.parse().unwrap(),
                    &mut write_avoidance_buffer,
                )
                .unwrap();
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

        let mut write_avoidance_buffer = Vec::new();
        for (url, timestamp, content) in docs {
            let mut write = repo
                .create(
                    url.parse().unwrap(),
                    timestamp.parse().unwrap(),
                    &mut write_avoidance_buffer,
                )
                .unwrap();
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

        DocRepo::new(path).unwrap()
    }
}
