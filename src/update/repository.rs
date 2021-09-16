use super::*;
use chrono::{DateTime, FixedOffset};
use io::Read;
use std::{
    cmp::max,
    fs::{self},
    io::{self, Write},
    iter,
    path::{Path, PathBuf},
};

pub struct UpdateRepo {
    base: PathBuf,
}

impl UpdateRepo {
    pub fn new(base: impl AsRef<Path>) -> io::Result<Self> {
        let base = base.as_ref().to_path_buf();
        fs::create_dir_all(&base)?;
        Ok(Self { base })
    }

    pub fn create(
        &self,
        url: Url,
        timestamp: DateTime<FixedOffset>,
        change: &str,
    ) -> io::Result<(Update, impl Iterator<Item = UpdateEvent>)> {
        let path = self.path_for(&url, Some(&timestamp));
        let update = Update::new(url, timestamp, change.to_owned());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(update.change.as_bytes())?;
        file.flush()?;

        let events = iter::once(UpdateEvent::Added {
            url: update.url().clone(),
            timestamp,
        })
        .chain(if self.latest(update.url())? == timestamp {
            Some(UpdateEvent::New {
                url: update.url().clone(),
                timestamp,
            })
        } else {
            None
        });
        Ok((update, events))
    }

    pub fn ensure(
        &self,
        url: Url,
        timestamp: DateTime<FixedOffset>,
        change: &str,
    ) -> io::Result<(Update, impl Iterator<Item = UpdateEvent>)> {
        let path = self.path_for(&url, Some(&timestamp));
        let update = Update::new(url, timestamp, change.to_owned());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Ok(mut file) = fs::OpenOptions::new().read(true).open(&path) {
            let mut contents = String::new();
            file.read_to_string(&mut contents)?;
            if change == contents {
                return Ok((update, vec![].into_iter()));
            }
        }

        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        file.write_all(update.change.as_bytes())?;
        file.flush()?;

        let added_event = UpdateEvent::Added {
            url: update.url().clone(),
            timestamp,
        };
        let events = if self.latest(update.url())? == timestamp {
            vec![
                added_event,
                UpdateEvent::New {
                    url: update.url().clone(),
                    timestamp,
                },
            ]
        } else {
            vec![added_event]
        };
        Ok((update, events.into_iter()))
    }

    /// Returns error if there is no update
    pub fn latest(&self, url: &Url) -> io::Result<DateTime<FixedOffset>> {
        let dir = fs::read_dir(self.path_for(url, None))?;
        let mut latest = None;
        for entry in dir {
            let entry = entry?;
            let timestamp: DateTime<FixedOffset> = entry
                .file_name()
                .to_str()
                .ok_or_else::<io::Error, _>(|| io::ErrorKind::Other.into())?
                .parse()
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
            if let Some(latest_i) = latest {
                latest = Some(max(latest_i, timestamp));
            } else {
                latest = Some(timestamp);
            }
        }
        latest.ok_or_else(|| io::ErrorKind::NotFound.into())
    }

    pub fn get_update(&self, url: Url, timestamp: DateTime<FixedOffset>) -> io::Result<Update> {
        let mut file = fs::File::open(self.path_for(&url, Some(&timestamp)))?;
        let mut change = vec![];
        file.read_to_end(&mut change)?;
        let change = String::from_utf8(change).map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        let doc_version = Update::new(url, timestamp, change);
        Ok(doc_version)
    }

    /// Lists all updates on the specified url from newest to oldest
    pub fn list_updates(&self, url: Url) -> io::Result<impl DoubleEndedIterator<Item = io::Result<Update>> + '_> {
        let mut dir: Vec<fs::DirEntry> = fs::read_dir(self.path_for(&url, None))?.collect::<io::Result<_>>()?;
        dir.sort_by_key(fs::DirEntry::file_name);

        Ok(dir.into_iter().rev().map(move |dir_entry| {
            let timestamp = dir_entry
                .file_name()
                .to_str()
                .unwrap()
                .parse()
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
            let change = String::from_utf8(fs::read(&self.path_for(&url, Some(&timestamp)))?)
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
            Ok(Update::new(url.clone(), timestamp, change))
        }))
    }

    /// Lists all updates
    pub fn list_all(&self, base_url: &Url) -> io::Result<IterDocs<'_>> {
        let root_path: PathBuf = base_url.to_path(&self.base);
        Ok(IterDocs {
            repo: self,
            url: base_url.clone(),
            stack: vec![fs::read_dir(root_path)?],
        })
    }

    fn path_for(&self, url: &Url, timestamp: Option<&DateTime<FixedOffset>>) -> PathBuf {
        let path = url.to_path(&self.base);
        if let Some(timestamp) = timestamp {
            path.join(timestamp.to_rfc3339())
        } else {
            path
        }
    }
}

// iterator over all updates in the repo
pub struct IterDocs<'r> {
    repo: &'r UpdateRepo,
    url: Url,
    stack: Vec<fs::ReadDir>,
}

impl<'r> Iterator for IterDocs<'r> {
    type Item = io::Result<Update>;

    fn next(&mut self) -> Option<Self::Item> {
        // ascend the tree if at the end of branches and get the next `DirEntry`
        let mut next_dir_entry = loop {
            if let Some(iter) = self.stack.last_mut() {
                if let Some(entry) = iter.next() {
                    break entry;
                } else {
                    self.stack.pop().unwrap();
                    self.url.pop_path_segment();
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
                        self.url.push_path_segment(dir_entry.file_name().to_str().unwrap());
                        let mut dir = fs::read_dir(self.repo.path_for(&self.url, None)).unwrap();
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
                        let change = fs::read_to_string(dir_entry.path()).unwrap();
                        break Some(Ok(Update::new(url, timestamp, change)));
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
    use std::{thread, time};

    use chrono::Utc;

    use super::*;

    #[test]
    fn old_update_creates_events_and_becomes_available() {
        let repo = test_repo("new_update_creates_events_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let timestamp = DateTime::<FixedOffset>::from(Utc::now()) - chrono::Duration::minutes(60);
        let change = "older change";
        let should = Update::new(url.clone(), timestamp, change.to_owned());

        let (_, events) = repo.create(url.clone(), Utc::now().into(), "newest change").unwrap();
        thread::sleep(time::Duration::from_millis(1));
        assert_eq!(events.count(), 2);

        let (update, events) = repo.create(url.clone(), timestamp, change).unwrap();
        assert_eq!(update, should);

        let update: Update = repo.get_update(url.clone(), timestamp).unwrap();
        assert_eq!(update, should);

        let mut list = repo.list_updates(url.clone()).unwrap();
        let update = list.next().unwrap().unwrap();
        assert_eq!(update.change, "newest change");
        let update = list.next().unwrap().unwrap();
        assert_eq!(update, should);

        thread::sleep(time::Duration::from_millis(1));
        assert_eq!(
            events.collect::<Vec<_>>(),
            [UpdateEvent::Added {
                url: url.clone(),
                timestamp
            }]
        );
    }

    #[test]
    fn newer_update_creates_event_and_becomes_available() {
        let repo = test_repo("newer_update_creates_event_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let change = "new change";
        let timestamp = Utc::now().into();
        let should = Update::new(url.clone(), timestamp, change.to_owned());

        let (_, events) = repo
            .create(
                url.clone(),
                DateTime::<FixedOffset>::from(Utc::now()) - chrono::Duration::minutes(60),
                "old change",
            )
            .unwrap();
        thread::sleep(time::Duration::from_millis(1));
        assert_eq!(events.count(), 2);

        let (update, events) = repo.create(url.clone(), timestamp, change).unwrap();
        assert_eq!(update, should);

        let update: Update = repo.get_update(url.clone(), timestamp).unwrap();
        assert_eq!(update, should);

        let mut list = repo.list_updates(url.clone()).unwrap();
        let update = list.next().unwrap().unwrap();
        assert_eq!(update, should);
        let update = list.next().unwrap().unwrap();
        assert_eq!(update.change, "old change");

        thread::sleep(time::Duration::from_millis(1));
        assert_eq!(
            events.collect::<Vec<_>>(),
            [
                UpdateEvent::Added {
                    url: url.clone(),
                    timestamp
                },
                UpdateEvent::New {
                    url: url.clone(),
                    timestamp
                }
            ]
        );
    }

    #[test]
    fn old_update_ensure_creates_events_and_becomes_available() {
        let repo = test_repo("old_update_ensure_creates_events_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let timestamp = DateTime::<FixedOffset>::from(Utc::now()) - chrono::Duration::minutes(60);
        let change = "older change";
        let should = Update::new(url.clone(), timestamp, change.to_owned());

        let (_, events) = repo.ensure(url.clone(), Utc::now().into(), "newest change").unwrap();
        thread::sleep(time::Duration::from_millis(1));
        assert_eq!(events.count(), 2);

        let (update, events) = repo.ensure(url.clone(), timestamp, change).unwrap();
        assert_eq!(update, should);

        let update: Update = repo.get_update(url.clone(), timestamp).unwrap();
        assert_eq!(update, should);

        let mut list = repo.list_updates(url.clone()).unwrap();
        let update = list.next().unwrap().unwrap();
        assert_eq!(update.change, "newest change");
        let update = list.next().unwrap().unwrap();
        assert_eq!(update, should);

        thread::sleep(time::Duration::from_millis(1));
        assert_eq!(
            events.collect::<Vec<_>>(),
            [UpdateEvent::Added {
                url: url.clone(),
                timestamp
            }]
        );
    }

    #[test]
    fn newer_update_ensure_creates_event_and_becomes_available() {
        let repo = test_repo("newer_update_ensure_creates_event_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let change = "new change";
        let timestamp = Utc::now().into();
        let should = Update::new(url.clone(), timestamp, change.to_owned());

        let (_, events) = repo
            .ensure(
                url.clone(),
                DateTime::<FixedOffset>::from(Utc::now()) - chrono::Duration::minutes(60),
                "old change",
            )
            .unwrap();
        thread::sleep(time::Duration::from_millis(1));
        assert_eq!(events.count(), 2);

        let (update, events) = repo.ensure(url.clone(), timestamp, change).unwrap();
        assert_eq!(update, should);

        let update: Update = repo.get_update(url.clone(), timestamp).unwrap();
        assert_eq!(update, should);

        let mut list = repo.list_updates(url.clone()).unwrap();
        let update = list.next().unwrap().unwrap();
        assert_eq!(update, should);
        let update = list.next().unwrap().unwrap();
        assert_eq!(update.change, "old change");

        thread::sleep(time::Duration::from_millis(1));
        assert_eq!(
            events.collect::<Vec<_>>(),
            [
                UpdateEvent::Added {
                    url: url.clone(),
                    timestamp
                },
                UpdateEvent::New {
                    url: url.clone(),
                    timestamp
                }
            ]
        );
    }

    #[test]
    fn existing_update_ensure_is_noop() {
        let repo = test_repo("existing_update_ensure_is_noop");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let change = "existing change";
        let timestamp = Utc::now().into();
        let should = Update::new(url.clone(), timestamp, change.to_owned());

        let (_, events) = repo.ensure(url.clone(), timestamp, change).unwrap();
        thread::sleep(time::Duration::from_millis(1));
        assert_eq!(events.count(), 2);

        let (update, events) = repo.ensure(url.clone(), timestamp, change).unwrap();
        assert_eq!(update, should);

        let update: Update = repo.get_update(url.clone(), timestamp).unwrap();
        assert_eq!(update, should);

        let mut list = repo.list_updates(url.clone()).unwrap();
        let update = list.next().unwrap().unwrap();
        assert_eq!(update, should);
        assert!(list.next().is_none());

        thread::sleep(time::Duration::from_millis(1));
        assert_eq!(events.count(), 0);
    }

    #[test]
    fn list_updates() {
        let repo = test_repo("list_updates");

        let docs = &[
            ("http://www.example.org/test/doc1", "2021-03-01T10:00:00+00:00", "1"),
            ("http://www.example.org/test/doc1", "2021-03-01T11:00:00+00:00", "2"),
            ("http://www.example.org/test/doc1", "2021-03-01T12:00:00+00:00", "3"),
            ("http://www.example.org/test/doc2", "2021-03-01T11:00:00+00:00", "4"),
            ("http://www.example.org/test/doc2", "2021-03-01T12:00:00+00:00", "5"),
        ];

        for (url, timestamp, content) in docs {
            let _ = repo
                .create(url.parse().unwrap(), timestamp.parse().unwrap(), content)
                .unwrap();
        }

        let result = repo
            .list_updates("http://www.example.org/test/doc1".parse().unwrap())
            .unwrap();
        for (&(e_url, e_ts, e_content), actual) in docs[0..3].iter().rev().zip(result) {
            let actual = actual.unwrap();
            assert_eq!(actual.url().as_str(), e_url);
            assert_eq!(actual.timestamp().to_rfc3339(), e_ts);
            assert_eq!(actual.change(), e_content);
        }
    }

    #[test]
    fn list_all() {
        let repo = test_repo("list_all");

        let docs = &[
            ("http://www.example.org/test/doc1", "2021-03-01T10:00:00+00:00", "1"),
            ("http://www.example.org/test/doc1", "2021-03-01T11:00:00+00:00", "2"),
            ("http://www.example.org/test/doc1", "2021-03-01T12:00:00+00:00", "3"),
            ("http://www.example.org/test/doc2", "2021-03-01T11:00:00+00:00", "4"),
            ("http://www.example.org/test/doc2", "2021-03-01T12:00:00+00:00", "5"),
        ];

        for (url, timestamp, content) in docs {
            let _ = repo
                .create(url.parse().unwrap(), timestamp.parse().unwrap(), content)
                .unwrap();
        }

        let result = repo.list_all(&"http://www.example.org/".parse().unwrap()).unwrap();
        for (&(e_url, e_ts, e_content), actual) in docs.iter().zip(result) {
            let actual = actual.unwrap();
            assert_eq!(actual.url().as_str(), e_url);
            assert_eq!(actual.timestamp().to_rfc3339(), e_ts);
            assert_eq!(actual.change(), e_content);
        }
    }

    fn test_repo(name: &str) -> UpdateRepo {
        let path = format!("tmp/{}", name);
        let _ = fs::remove_dir_all(&path);
        UpdateRepo::new(path).unwrap()
    }
}
