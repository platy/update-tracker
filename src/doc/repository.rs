use super::*;
use chrono::{DateTime, Utc};
use std::{
    ffi::OsStr,
    fs,
    io::{self, Write},
    iter,
    path::{self, Path, PathBuf},
};

pub struct DocRepo {
    base: PathBuf,
}

impl DocRepo {
    pub fn new(base: impl AsRef<Path>) -> io::Result<Self> {
        let base = base.as_ref().to_path_buf();
        fs::create_dir_all(&base)?;
        Ok(Self { base })
    }

    pub fn create(&self, url: Url, timestamp: DateTime<Utc>) -> io::Result<TempDoc> {
        let doc = DocumentVersion { url, timestamp };
        let path = self.path_for_version(&doc);
        let is_new_doc = !self.document_exists(&doc.url)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
        Ok(TempDoc { is_new_doc, doc, file })
    }

    pub fn open(&self, doc_version: &DocumentVersion) -> io::Result<impl io::Read> {
        fs::File::open(self.path_for_version(doc_version))
    }

    pub fn ensure_version(&self, url: Url, timestamp: DateTime<Utc>) -> io::Result<DocumentVersion> {
        let doc_version = DocumentVersion { url, timestamp };
        fs::File::open(self.path_for_version(&doc_version))?;
        Ok(doc_version)
    }

    /// Lists all updates on the specified url from newest to oldest
    pub fn list_versions(&self, url: Url) -> io::Result<impl Iterator<Item = io::Result<DocumentVersion>>> {
        let doc = Document { url: url.clone() };
        let mut dir: Vec<fs::DirEntry> = fs::read_dir(self.path_for_doc(&doc))?.collect::<io::Result<_>>()?;
        dir.sort_by_key(fs::DirEntry::file_name);

        Ok(dir.into_iter().rev().map(move |dir_entry| {
            let timestamp = dir_entry
                .file_name()
                .to_str()
                .unwrap()
                .parse()
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
            Ok(DocumentVersion {
                url: url.clone(),
                timestamp,
            })
        }))
    }

    /// Lists all updates
    pub fn list_all(&self, base_url: &Url) -> io::Result<IterDocs<'_>> {
        let root_path: PathBuf = self
            .base
            .components()
            .chain(iter::once(path::Component::Normal(OsStr::new(
                base_url.host_str().unwrap(),
            ))))
            .collect();
        Ok(IterDocs {
            repo: self,
            url: base_url.clone(),
            stack: vec![fs::read_dir(root_path)?],
        })
    }

    pub fn document_exists(&self, url: &Url) -> io::Result<bool> {
        match fs::read_dir(self.path_for_doc(&Document { url: url.clone() })) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Ok(mut iter) => Ok(iter.next().is_some()),
            Err(err) => Err(err),
        }
    }

    fn path_for_doc(&self, Document { url }: &Document) -> PathBuf {
        let path = url.path().strip_prefix('/').unwrap_or_else(|| url.path());
        self.base.join(url.host_str().unwrap_or("local")).join(path)
    }

    fn path_for_version(&self, DocumentVersion { url, timestamp }: &DocumentVersion) -> PathBuf {
        let path = url.path().strip_prefix('/').unwrap_or_else(|| url.path());
        self.base
            .join(url.host_str().unwrap_or("local"))
            .join(path)
            .join(timestamp.to_rfc3339())
    }
}

/// TODO Maybe this should write to a temp file to start with and then be moved into place, that way the whole repo structure will consist of complete files
pub struct TempDoc {
    is_new_doc: bool, // TODO replace with something better when fixing the above
    doc: DocumentVersion,
    file: fs::File,
}

impl TempDoc {
    pub fn done(mut self) -> io::Result<(DocumentVersion, impl Iterator<Item = DocEvent>)> {
        self.file.flush()?;
        let events = if self.is_new_doc {
            Some(DocEvent::Created {
                url: self.doc.url.clone(),
            })
        } else {
            None
        }
        .into_iter()
        .chain(iter::once(DocEvent::Updated {
            url: self.doc.url.clone(),
            timestamp: self.doc.timestamp,
        }));
        Ok((self.doc, events))
    }
}

impl io::Write for TempDoc {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

// iterator over all docs in the repo
pub struct IterDocs<'r> {
    repo: &'r DocRepo,
    url: Url,
    stack: Vec<fs::ReadDir>,
}

impl<'r> Iterator for IterDocs<'r> {
    type Item = io::Result<DocumentVersion>;

    fn next(&mut self) -> Option<Self::Item> {
        // ascend the tree if at the end of branches and get the next `DirEntry`
        let mut next_dir_entry = loop {
            if let Some(iter) = self.stack.last_mut() {
                if let Some(entry) = iter.next() {
                    break entry;
                } else {
                    self.stack.pop().unwrap();
                    self.url.path_segments_mut().unwrap().pop();
                }
            } else {
                return None;
            }
        };

        // descend to the next doc
        loop {
            match next_dir_entry {
                Err(err) => break Some(Err(err)),
                Ok(dir_entry) => {
                    let file_type = dir_entry.file_type().unwrap();
                    if file_type.is_dir() {
                        self.url
                            .path_segments_mut()
                            .unwrap()
                            .push(dir_entry.file_name().to_str().unwrap());
                        let mut dir =
                            fs::read_dir(self.repo.path_for_doc(&Document { url: self.url.clone() })).unwrap();
                        next_dir_entry = dir.next().expect("todo: handle empty dir in repo");
                        self.stack.push(dir);
                    } else if file_type.is_file() {
                        let timestamp = dir_entry
                            .file_name()
                            .to_str()
                            .unwrap()
                            .parse()
                            .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
                            .unwrap();
                        let url = self.url.clone();
                        break Some(Ok(DocumentVersion {
                            url,
                            timestamp,
                        }));
                    } else {
                        panic!("symlink in repo");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::{
        io::Read,
        str::FromStr,
        thread::{self},
        time,
    };

    use super::*;

    #[test]
    fn new_doc_creates_events_and_becomes_available() {
        let repo = test_repo("new_doc_creates_events_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let doc_content = "test document";
        let timestamp = Utc::now();
        let should = DocumentVersion {
            url: url.clone(),
            timestamp,
        };
        let mut buf = vec![];

        let mut write = repo.create(url.clone(), timestamp).unwrap();
        write.write_all(doc_content.as_bytes()).unwrap();

        let (doc, events) = write.done().unwrap();
        assert_eq!(doc, should);
        repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, doc_content.as_bytes());

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

        thread::sleep(time::Duration::from_millis(1));
        let events: Vec<_> = events.collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], DocEvent::Created { url: url.clone() });
        assert_eq!(events[1], DocEvent::Updated { url, timestamp });
    }

    #[test]
    fn updated_doc_creates_event_and_becomes_available() {
        let repo = test_repo("updated_doc_creates_event_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let doc_content = "new content";
        let timestamp = Utc::now();
        let should = DocumentVersion {
            url: url.clone(),
            timestamp,
        };
        let mut buf = vec![];

        let mut write = repo
            .create(url.clone(), Utc::now() - chrono::Duration::seconds(60))
            .unwrap();
        write.write_all("old content".as_bytes()).unwrap();
        let (doc, _events) = write.done().unwrap();
        repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();
        thread::sleep(time::Duration::from_millis(1));

        let mut write = repo.create(url.clone(), timestamp).unwrap();
        write.write_all(doc_content.as_bytes()).unwrap();
        let (doc, events) = write.done().unwrap();

        assert_eq!(doc, should);
        buf.clear();
        repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, doc_content.as_bytes());

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

        thread::sleep(time::Duration::from_millis(1));
        let events: Vec<_> = events.collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], DocEvent::Updated { url, timestamp });
    }

    fn test_repo(name: &str) -> DocRepo {
        let path = format!("tmp/{}", name);
        let _ = fs::remove_dir_all(&path);
        let repo = DocRepo::new(path, Url::from_str("http://example.com").unwrap()).unwrap();
        repo
    }
}
