use super::*;
use chrono::{DateTime, Utc};
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
        timestamp: DateTime<Utc>,
        change: &str,
    ) -> io::Result<(Update, impl Iterator<Item = UpdateEvent>)> {
        let path = self.path_for(&url, Some(&timestamp));
        let update = Update {
            url,
            timestamp,
            change: change.to_owned(),
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(update.change.as_bytes())?;
        file.flush()?;

        let events = iter::once(UpdateEvent::Added {
            url: update.url.clone(),
            timestamp,
        })
        .chain(if self.latest(&update.url)? == timestamp {
            Some(UpdateEvent::New {
                url: update.url.clone(),
                timestamp,
            })
        } else {
            None
        });
        Ok((update, events))
    }

    /// Returns error if there is no update
    pub fn latest(&self, url: &Url) -> io::Result<DateTime<Utc>> {
        let dir = fs::read_dir(self.path_for(&url, None))?;
        let mut latest = None;
        for entry in dir {
            let entry = entry?;
            let timestamp: DateTime<Utc> = entry
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

    pub fn get_update(&self, url: Url, timestamp: DateTime<Utc>) -> io::Result<Update> {
        let mut file = fs::File::open(self.path_for(&url, Some(&timestamp)))?;
        let mut change = vec![];
        file.read_to_end(&mut change)?;
        let change = String::from_utf8(change).map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        let doc_version = Update { url, timestamp, change };
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
            Ok(Update {
                url: url.clone(),
                timestamp,
                change,
            })
        }))
    }

    fn path_for(&self, url: &Url, timestamp: Option<&DateTime<Utc>>) -> PathBuf {
        let path = url.path().strip_prefix('/').unwrap_or_else(|| url.path());
        let path = self.base.join(url.host_str().unwrap_or("local")).join(path);
        if let Some(timestamp) = timestamp {
            path.join(timestamp.to_rfc3339())
        } else {
            path
        }
    }
}

#[cfg(test)]
mod test {
    use std::{thread, time};

    use super::*;

    #[test]
    fn old_update_creates_events_and_becomes_available() {
        let repo = test_repo("new_update_creates_events_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let timestamp = Utc::now() - chrono::Duration::minutes(60);
        let change = "older change";
        let should = Update {
            url: url.clone(),
            timestamp,
            change: change.to_owned(),
        };

        let (_, events) = repo.create(url.clone(), Utc::now(), "newest change").unwrap();
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
        let timestamp = Utc::now();
        let should = Update {
            url: url.clone(),
            timestamp,
            change: change.to_owned(),
        };

        let (_, events) = repo
            .create(url.clone(), Utc::now() - chrono::Duration::minutes(60), "old change")
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

    fn test_repo(name: &str) -> UpdateRepo {
        let path = format!("tmp/{}", name);
        let _ = fs::remove_dir_all(&path);
        UpdateRepo::new(path).unwrap()
    }
}
