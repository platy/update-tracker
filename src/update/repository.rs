use super::*;
use chrono::{DateTime, Utc};
use io::Read;
use std::{
    cmp::max,
    fs::{self},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::mpsc,
};

pub struct UpdateRepo {
    base: PathBuf,
    events: mpsc::Sender<UpdateEvent>,
}

impl UpdateRepo {
    pub fn new(base: impl AsRef<Path>, events: mpsc::Sender<UpdateEvent>) -> io::Result<Self> {
        let base = base.as_ref().to_path_buf();
        fs::create_dir_all(&base)?;
        Ok(Self { base, events })
    }

    pub fn create(&self, url: Url, timestamp: DateTime<Utc>, change: &str) -> io::Result<Update> {
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

        self.events
            .send(UpdateEvent::Added {
                url: update.url.clone(),
                timestamp,
            })
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        if self.latest(&update.url)? == timestamp {
            self.events
                .send(UpdateEvent::New {
                    url: update.url.clone(),
                    timestamp,
                })
                .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        }
        Ok(update)
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
    use std::{sync, thread, time};

    use super::*;

    #[test]
    fn old_update_creates_events_and_becomes_available() {
        let (repo, events) = test_repo("new_update_creates_events_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let timestamp = Utc::now() - chrono::Duration::minutes(60);
        let change = "older change";
        let should = Update {
            url: url.clone(),
            timestamp,
            change: change.to_owned(),
        };

        let _ = repo.create(url.clone(), Utc::now(), "newest change").unwrap();
        thread::sleep(time::Duration::from_millis(1));
        assert_eq!(events.lock().unwrap().drain(..).count(), 2);

        let update = repo.create(url.clone(), timestamp, change).unwrap();
        assert_eq!(update, should);

        let update: Update = repo.get_update(url.clone(), timestamp).unwrap();
        assert_eq!(update, should);

        let mut list = repo.list_updates(url.clone()).unwrap();
        let update = list.next().unwrap().unwrap();
        assert_eq!(update.change, "newest change");
        let update = list.next().unwrap().unwrap();
        assert_eq!(update, should);

        thread::sleep(time::Duration::from_millis(1));
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            UpdateEvent::Added {
                url: url.clone(),
                timestamp
            }
        );
    }

    #[test]
    fn newer_update_creates_event_and_becomes_available() {
        let (repo, events) = test_repo("newer_update_creates_event_and_becomes_available");
        let url: Url = "http://www.example.org/test/doc".parse().unwrap();
        let change = "new change";
        let timestamp = Utc::now();
        let should = Update {
            url: url.clone(),
            timestamp,
            change: change.to_owned(),
        };

        repo.create(url.clone(), Utc::now() - chrono::Duration::minutes(60), "old change")
            .unwrap();
        thread::sleep(time::Duration::from_millis(1));
        assert_eq!(events.lock().unwrap().drain(..).count(), 2);

        let update = repo.create(url.clone(), timestamp, change).unwrap();
        assert_eq!(update, should);

        let update: Update = repo.get_update(url.clone(), timestamp).unwrap();
        assert_eq!(update, should);

        let mut list = repo.list_updates(url.clone()).unwrap();
        let update = list.next().unwrap().unwrap();
        assert_eq!(update, should);
        let update = list.next().unwrap().unwrap();
        assert_eq!(update.change, "old change");

        thread::sleep(time::Duration::from_millis(1));
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            UpdateEvent::Added {
                url: url.clone(),
                timestamp
            }
        );
        assert_eq!(
            events[1],
            UpdateEvent::New {
                url: url.clone(),
                timestamp
            }
        );
    }

    fn test_repo(name: &str) -> (UpdateRepo, sync::Arc<sync::Mutex<Vec<UpdateEvent>>>) {
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
        let repo = UpdateRepo::new(path, event_sender).unwrap();
        (repo, events1)
    }
}
