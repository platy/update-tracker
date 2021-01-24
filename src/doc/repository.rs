use super::*;
use chrono::{format::parse, DateTime, Utc};
use mpsc::Sender;
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::mpsc,
};

struct DocRepo {
    base: PathBuf,
    events: mpsc::Sender<DocEvent>,
}

impl DocRepo {
    fn new(base: impl AsRef<Path>, events: mpsc::Sender<DocEvent>) -> io::Result<DocRepo> {
        let base = base.as_ref().to_path_buf();
        fs::create_dir_all(&base)?;
        Ok(DocRepo {
            base,
            events,
        })
    }

    fn create(&self, url: Url, timestamp: DateTime<Utc>) -> io::Result<TempDoc> {
        let doc = DocumentVersion { url, timestamp };
        let path = self.path_for_version(&doc);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        Ok(TempDoc { doc, file, events: self.events.clone() })
    }

    fn open(&self, doc_version: &DocumentVersion) -> io::Result<impl io::Read> {
        fs::File::open(self.path_for_version(doc_version))
    }

    fn ensure_version(&self, url: Url, timestamp: DateTime<Utc>) -> io::Result<DocumentVersion> {
        let doc_version = DocumentVersion { url, timestamp };
        fs::File::open(self.path_for_version(&doc_version))?;
        Ok(doc_version)
    }

    fn list_versions(&self, url: Url) -> io::Result<impl Iterator<Item = io::Result<DocumentVersion>>> {
        let doc = Document { url: url.clone() };
        let dir = fs::read_dir(self.path_for_doc(&doc))?;
        Ok(dir.map(move |dir_result| {
            dir_result.and_then(|dir_entry| {
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
            })
        }))
    }

    fn path_for_doc(&self, Document { url }: &Document) -> PathBuf {
        let path = url.path().strip_prefix('/').unwrap_or(url.path());
        self.base.join(url.host_str().unwrap_or("local")).join(path)
    }

    fn path_for_version(&self, DocumentVersion { url, timestamp }: &DocumentVersion) -> PathBuf {
        let path = url.path().strip_prefix('/').unwrap_or(url.path());
        self.base
            .join(url.host_str().unwrap_or("local"))
            .join(path)
            .join(timestamp.to_rfc3339())
    }
}

/// TODO Maybe this should write to a temp file to start with and then be moved into place, that way the whole repo structure will consist of complete files
struct TempDoc {
    doc: DocumentVersion,
    file: fs::File,
    events: mpsc::Sender<DocEvent>,
}

impl TempDoc {
    fn done(mut self) -> io::Result<DocumentVersion> {
        self.file.flush()?;
        // TODO - this is not always creating, add antoher test and fix for update
        self.events.send(DocEvent::Created { url: self.doc.url.clone() }).map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        self.events.send(DocEvent::Updated { url: self.doc.url.clone(), timestamp: self.doc.timestamp }).map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        Ok(self.doc)
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

#[cfg(test)]
mod test {
    use std::{
        io::Read,
        sync,
        thread::{self, spawn},
    };

    use super::*;

    #[test]
    fn new_doc_creates_events_and_becomes_available() {
        let (repo, events) = test_repo("new_doc_creates_events_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let doc_content = "test document";
        let timestamp = Utc::now();
        let mut buf = vec![];

        let mut write = repo.create(url.clone(), timestamp).unwrap();
        write.write_all(doc_content.as_bytes()).unwrap();

        let doc: DocumentVersion = write.done().unwrap();
        assert_eq!(
            doc,
            DocumentVersion {
                url: url.clone(),
                timestamp
            }
        );
        repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, doc_content.as_bytes());

        let doc: DocumentVersion = repo.ensure_version(url.clone(), timestamp).unwrap();
        assert_eq!(
            doc,
            DocumentVersion {
                url: url.clone(),
                timestamp
            }
        );
        buf.clear();
        repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, doc_content.as_bytes());

        let mut docs = repo.list_versions(url.clone()).unwrap();
        let doc = docs.next().unwrap().unwrap();
        assert_eq!(
            doc,
            DocumentVersion {
                url: url.clone(),
                timestamp
            }
        );
        buf.clear();
        repo.open(&doc).unwrap().read_to_end(&mut buf).unwrap();
        assert_eq!(buf, doc_content.as_bytes());

        let events = events.lock().unwrap();
        assert_eq!(events[0], DocEvent::Created { url: url.clone() });
        assert_eq!(
            events[1],
            DocEvent::Updated {
                url: url.clone(),
                timestamp
            }
        );
    }

    fn test_repo(name: &str) -> (DocRepo, sync::Arc<sync::Mutex<Vec<DocEvent>>>) {
        let events = sync::Arc::new(sync::Mutex::new(vec![]));
        let events1 = events.clone();
        let (event_sender, event_receiver) = mpsc::channel();
        thread::spawn(move || {
            while let Ok(event) = event_receiver.recv() {
                events.lock().unwrap().push(event);
            }
        });
        let path = format!("tmp/{}", name);
        let _ = fs::remove_dir_all(&path);
        let repo = DocRepo::new(path, event_sender).unwrap();
        (repo, events1)
    }
}
