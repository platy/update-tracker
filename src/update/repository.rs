use super::*;
use crate::{
    repository::*,
    url::{IterUrlRepoLeaves, UrlRepo},
};

use chrono::{DateTime, FixedOffset};
use io::Read;
use std::{
    cmp::max,
    fs::{self},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub struct UpdateRepo {
    repo: UrlRepo,
}

impl UpdateRepo {
    pub fn new(base: impl AsRef<Path>) -> io::Result<Self> {
        let repo = UrlRepo::new("update", base)?;
        Ok(Self { repo })
    }

    pub fn create(&self, url: Url, timestamp: DateTime<FixedOffset>, change: &str) -> WriteResult<Update, 2> {
        let path = self.path_for(&url, Some(&timestamp));
        let update = Update::new(url, timestamp, change.to_owned());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(update.change.as_bytes())?;
        file.flush()?;

        let is_latest = self.latest(update.url())? == timestamp;
        let events = [
            Some(UpdateEvent::added(&update)),
            is_latest.then(|| UpdateEvent::new(&update)),
        ];
        update.with_events(events)
    }

    pub fn ensure(&self, url: Url, timestamp: DateTime<FixedOffset>, change: &str) -> WriteResult<Update, 2> {
        let path = self.path_for(&url, Some(&timestamp));
        let update = Update::new(url, timestamp, change.to_owned());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Ok(mut file) = fs::OpenOptions::new().read(true).open(&path) {
            let mut contents = String::new();
            file.read_to_string(&mut contents)?;
            if change == contents {
                return update.with_events(Default::default());
            }
        }

        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        file.write_all(update.change.as_bytes())?;
        file.flush()?;

        let is_latest = self.latest(update.url())? == timestamp;
        let events = [
            Some(UpdateEvent::added(&update)),
            is_latest.then(|| UpdateEvent::new(&update)),
        ];
        update.with_events(events)
    }

    /// Returns error if there is no update
    pub fn latest(&self, url: &Url) -> io::Result<DateTime<FixedOffset>> {
        let dir = self.repo.read_leaves_for_url(url)?;
        let mut latest = None;
        for entry in dir {
            let (name, _) = entry?;
            let timestamp: DateTime<FixedOffset> = name
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
        let files = self.repo.read_leaves_sorted_for_url(&url)?;

        Ok(files.rev().map(move |dir_entry| {
            let timestamp = dir_entry
                .0
                .parse()
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
            let change = String::from_utf8(fs::read(&self.path_for(&url, Some(&timestamp)))?)
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error))?;
            Ok(Update::new(url.clone(), timestamp, change))
        }))
    }

    /// Lists all updates
    pub fn list_all(&self, base_url: &Url) -> io::Result<IterUrlRepoLeaves<'_, Update>> {
        self.repo.list_all(base_url.clone(), |url, name, dir_entry| {
            let timestamp = name
                .parse()
                .map_err(|error| io::Error::new(io::ErrorKind::Other, error))
                .unwrap();
            let change = fs::read_to_string(dir_entry.path()).unwrap();
            Update {
                update_ref: UpdateRef { url, timestamp },
                change,
            }
        })
    }

    fn path_for(&self, url: &Url, timestamp: Option<&DateTime<FixedOffset>>) -> PathBuf {
        if let Some(timestamp) = timestamp {
            self.repo.leaf_path(url, &timestamp.to_rfc3339())
        } else {
            self.repo.node_path(url)
        }
    }
}

#[cfg(test)]
mod test {
    use chrono::Utc;

    use super::*;

    #[test]
    fn old_update_creates_events_and_becomes_available() {
        let repo = test_repo("update::new_update_creates_events_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let timestamp = DateTime::<FixedOffset>::from(Utc::now()) - chrono::Duration::minutes(60);
        let change = "older change";
        let should = Update::new(url.clone(), timestamp, change.to_owned());

        let update = repo.create(url.clone(), Utc::now().into(), "newest change").unwrap();
        assert_eq!(update.into_events().count(), 2);

        let update = repo.create(url.clone(), timestamp, change).unwrap();
        assert_eq!(*update, should);

        assert_eq!(
            update.into_events().collect::<Vec<_>>(),
            [UpdateEvent::Added {
                url: url.clone(),
                timestamp
            }]
        );

        let update: Update = repo.get_update(url.clone(), timestamp).unwrap();
        assert_eq!(update, should);

        let mut list = repo.list_updates(url.clone()).unwrap();
        let update = list.next().unwrap().unwrap();
        assert_eq!(update.change, "newest change");
        let update = list.next().unwrap().unwrap();
        assert_eq!(update, should);
    }

    #[test]
    fn newer_update_creates_event_and_becomes_available() {
        let repo = test_repo("update::newer_update_creates_event_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let change = "new change";
        let timestamp = Utc::now().into();
        let should = Update::new(url.clone(), timestamp, change.to_owned());

        let update = repo
            .create(
                url.clone(),
                DateTime::<FixedOffset>::from(Utc::now()) - chrono::Duration::minutes(60),
                "old change",
            )
            .unwrap();
        assert_eq!(update.into_events().count(), 2);

        let update = repo.create(url.clone(), timestamp, change).unwrap();
        assert_eq!(*update, should);
        assert_eq!(
            update.into_events().collect::<Vec<_>>(),
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

        let update: Update = repo.get_update(url.clone(), timestamp).unwrap();
        assert_eq!(update, should);

        let mut list = repo.list_updates(url.clone()).unwrap();
        let update = list.next().unwrap().unwrap();
        assert_eq!(update, should);
        let update = list.next().unwrap().unwrap();
        assert_eq!(update.change, "old change");
    }

    #[test]
    fn old_update_ensure_creates_events_and_becomes_available() {
        let repo = test_repo("update::old_update_ensure_creates_events_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let timestamp = DateTime::<FixedOffset>::from(Utc::now()) - chrono::Duration::minutes(60);
        let change = "older change";
        let should = Update::new(url.clone(), timestamp, change.to_owned());

        let update = repo.ensure(url.clone(), Utc::now().into(), "newest change").unwrap();
        assert_eq!(update.into_events().count(), 2);

        let update = repo.ensure(url.clone(), timestamp, change).unwrap();
        assert_eq!(*update, should);
        assert_eq!(
            update.into_events().collect::<Vec<_>>(),
            [UpdateEvent::Added {
                url: url.clone(),
                timestamp
            }]
        );

        let update: Update = repo.get_update(url.clone(), timestamp).unwrap();
        assert_eq!(update, should);

        let mut list = repo.list_updates(url.clone()).unwrap();
        let update = list.next().unwrap().unwrap();
        assert_eq!(update.change, "newest change");
        let update = list.next().unwrap().unwrap();
        assert_eq!(update, should);
    }

    #[test]
    fn newer_update_ensure_creates_event_and_becomes_available() {
        let repo = test_repo("update::newer_update_ensure_creates_event_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let change = "new change";
        let timestamp = Utc::now().into();
        let should = Update::new(url.clone(), timestamp, change.to_owned());

        let update = repo
            .ensure(
                url.clone(),
                DateTime::<FixedOffset>::from(Utc::now()) - chrono::Duration::minutes(60),
                "old change",
            )
            .unwrap();
        assert_eq!(update.into_events().count(), 2);

        let update = repo.ensure(url.clone(), timestamp, change).unwrap();
        assert_eq!(*update, should);
        assert_eq!(
            update.into_events().collect::<Vec<_>>(),
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

        let update: Update = repo.get_update(url.clone(), timestamp).unwrap();
        assert_eq!(update, should);

        let mut list = repo.list_updates(url.clone()).unwrap();
        let update = list.next().unwrap().unwrap();
        assert_eq!(update, should);
        let update = list.next().unwrap().unwrap();
        assert_eq!(update.change, "old change");
    }

    #[test]
    fn existing_update_ensure_is_noop() {
        let repo = test_repo("update::existing_update_ensure_is_noop");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let change = "existing change";
        let timestamp = Utc::now().into();
        let should = Update::new(url.clone(), timestamp, change.to_owned());

        let update = repo.ensure(url.clone(), timestamp, change).unwrap();
        assert_eq!(update.into_events().count(), 2);

        let update = repo.ensure(url.clone(), timestamp, change).unwrap();
        assert_eq!(*update, should);
        assert_eq!(update.into_events().count(), 0);

        let update: Update = repo.get_update(url.clone(), timestamp).unwrap();
        assert_eq!(update, should);

        let mut list = repo.list_updates(url.clone()).unwrap();
        let update = list.next().unwrap().unwrap();
        assert_eq!(update, should);
        assert!(list.next().is_none());
    }

    #[test]
    fn list_updates() {
        let repo = test_repo("update::list_updates");

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
        let repo = test_repo("update::list_all");

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
