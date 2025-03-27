use anyhow::{format_err, Context, Result};
use chrono::{Offset, TimeZone, Utc};
use std::{
    cell::RefCell,
    io::{self, copy, Write},
    sync::{Arc, RwLock},
};
use update_repo::{
    doc::{
        content::{Doc, DocContent},
        DocEvent, DocRepo,
    },
    tag::{TagEvent, TagRepo},
    update::UpdateRepo,
};
use ureq::get;
use url::Url;

pub mod email_update;

use self::email_update::GovUkChange;
use crate::data::Data;
use dotenv::dotenv;
use file_locker::FileLock;

use std::{
    collections::VecDeque,
    fs,
    io::Read,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

pub fn run(new_repo_path: &Path, data: Arc<RwLock<Data>>) -> Result<()> {
    let _ = dotenv();
    let govuk_emails_inbox = dotenv::var("INBOX")?;
    let outbox_dir = dotenv::var("OUTBOX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| new_repo_path.join("outbox"));
    let work_dir = dotenv::var("WORKDIR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| new_repo_path.join("work"));
    fs::create_dir_all(&govuk_emails_inbox).context(format!("Error trying to create dir {}", &govuk_emails_inbox))?;
    fs::create_dir_all(&outbox_dir).context(format!("Error trying to create dir {:?}", &outbox_dir))?;

    println!("Watching inbox {} for updates", &govuk_emails_inbox);

    let mut update_email_processor = UpdateEmailProcessor::new(
        govuk_emails_inbox.as_ref(),
        &outbox_dir,
        &work_dir,
        new_repo_path,
        &data,
    )?;
    loop {
        let count = update_email_processor
            .process_updates()
            .expect("the processing fails, the repo may be unclean");
        if count > 0 {
            println!("Processed {} update emails", count);
        }
        thread::sleep(Duration::from_secs(1));
    }
}

struct UpdateEmailProcessor<'a> {
    in_dir: &'a Path,
    out_dir: &'a Path,
    work_dir: &'a Path,
    new: NewRepoWriter<'a>,
}

impl<'a> UpdateEmailProcessor<'a> {
    fn new(
        in_dir: &'a Path,
        out_dir: &'a Path,
        work_dir: &'a Path,
        new_repo: &Path,
        data: &'a RwLock<Data>,
    ) -> Result<Self> {
        Ok(Self {
            in_dir,
            out_dir,
            work_dir,
            new: NewRepoWriter::new(new_repo, data)?,
        })
    }

    fn process_updates(&mut self) -> Result<u32> {
        let mut count = 0;
        for to_inbox in fs::read_dir(self.in_dir)? {
            let to_inbox = to_inbox?;
            if to_inbox.metadata()?.is_dir() {
                for email in fs::read_dir(to_inbox.path())? {
                    let email = email?;
                    println!("Processing {:?}", email);
                    if !(self
                        .process_email_update_file(to_inbox.file_name(), &email)
                        .context(format!(
                            "Failed processing {}",
                            email.path().to_str().unwrap_or_default()
                        ))?)
                    {
                        eprintln!(
                            "Non-fatal failure processing {}",
                            email.path().to_str().unwrap_or_default()
                        )
                    }
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    fn process_email_update_file(&mut self, to_dir_name: impl AsRef<Path>, dir_entry: &fs::DirEntry) -> Result<bool> {
        let working_path = self.work_dir.join(&to_dir_name).join(dir_entry.file_name());
        fs::create_dir_all(working_path.parent().unwrap()).context("Creating working dir")?;
        fs::copy(dir_entry.path(), &working_path).context(format!(
            "Copying file {} to {}",
            dir_entry.path().to_str().unwrap_or_default(),
            &working_path.to_str().unwrap_or_default()
        ))?;
        fs::remove_file(dir_entry.path()).context(format!(
            "Removing file {}",
            dir_entry.path().to_str().unwrap_or_default()
        ))?;
        let data = {
            let mut lock = FileLock::lock(working_path.to_str().context("error")?, true, false)
                .context("Locking file email file")?;
            let mut bytes = Vec::with_capacity(lock.file.metadata().map(|m| m.len() as usize + 1).unwrap_or(0));
            lock.file.read_to_end(&mut bytes).context("Reading email file")?;
            bytes
        };
        let updates = match GovUkChange::from_eml(&String::from_utf8(data)?) {
            Ok(updates) => updates,
            Err(err) => {
                eprintln!("Error parsing email: {:?}", &err);
                return Ok(false);
            }
        };
        for change in &updates {
            if let Err(err) = self.handle_change(change) {
                eprintln!("Error processing change: {:?}: {:?}", change, &err);
                return Ok(false);
            }
        }
        // successfully handled, 'commit' the new commits by updating the reference and then move email to outbox
        let done_path = self.out_dir.join(&to_dir_name).join(dir_entry.file_name());
        fs::create_dir_all(done_path.parent().unwrap()).context("Creating outbox dir")?;
        fs::rename(&working_path, &done_path).context(format!(
            "Renaming file {} to {}",
            working_path.to_str().unwrap_or_default(),
            &done_path.to_str().unwrap_or_default()
        ))?;
        Ok(true)
    }

    fn handle_change(
        &self,
        GovUkChange {
            url,
            change,
            updated_at,
            category,
        }: &GovUkChange,
    ) -> Result<()> {
        if let Err(err) = self.new.write_update(url, updated_at, change, category.as_deref()) {
            println!("Error writing to update repo {}", err);
        }

        for res in FetchDocs::fetch(url.clone()) {
            let (path, content) = res?;

            let mut url = url.clone();
            url.set_path(path.to_str().unwrap());
            let ts = Utc::now();
            let ts = ts.with_timezone(&ts.offset().fix());
            if let Err(err) = self.new.write_doc(url, ts, &content) {
                println!("Error writing to doc repo {}", err)
            }

            let mut path = path;
            if content.is_html() {
                assert!(path.set_extension("html"));
            }
        }
        Ok(())
    }
}

struct FetchDocs {
    urls: VecDeque<Url>,
}

impl FetchDocs {
    fn fetch(url: Url) -> FetchDocs {
        let mut urls = VecDeque::new();
        urls.push_back(url);
        Self { urls }
    }

    fn fetch_doc(&mut self, url: Url) -> Result<Option<(PathBuf, DocContent)>> {
        if let Some(doc) = retrieve_doc(&url).or_else(|err| {
            println!(
                "Request for {} failed with {}, waiting {:?} once and retrying",
                &url, err, RETRY_DELAY
            );
            thread::sleep(RETRY_DELAY);
            retrieve_doc(&url)
        })? {
            self.urls
                .extend(doc.content.attachments().unwrap_or_default().iter().cloned());
            let path = PathBuf::from(doc.url.path());
            println!("Writing doc to : {}", path.to_str().unwrap());
            Ok(Some((path, doc.content)))
        } else {
            Ok(None)
        }
    }
}

const RETRY_DELAY: Duration = Duration::from_secs(60);

impl Iterator for FetchDocs {
    type Item = Result<(PathBuf, DocContent)>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(url) = self.urls.pop_front() {
            if url.host_str() != Some("www.gov.uk") {
                println!("Ignoring link to offsite document : {}", &url);
                continue;
            }
            let doc = self.fetch_doc(url).transpose();
            if doc.is_some() {
                return doc;
            }
        }
        None
    }
}

/// Retrieve a document from the given URL
///
/// Returns None if the document is not found or has been deleted
pub fn retrieve_doc(url: &Url) -> Result<Option<Doc>> {
    println!("retrieving url : {}", url);
    let response = match get(url.as_str())
        .set("User-Agent", "GovDiffBot/0.1; +https://govdiff.njk.onl")
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::Status(410, _)) => return Ok(None), /* other responses could indicate that a retry should happen or that we have a programming issue, but 410 really means that we're requesting the intended document but it has been intentionally removed */
        err => err.context("Error retrieving")?,
    };

    if response.content_type() == "text/html" {
        let mut content = response.into_reader();
        let doc = Doc {
            content: DocContent::html(&mut content, Some(url)).map_err(|e| format_err!("Problem {}", e))?,
            url: url.to_owned(),
        };

        Ok(Some(doc))
    } else {
        let mut reader = response.into_reader();
        let mut buf = vec![];
        copy(&mut reader, &mut buf)
            .map_err(|err| format_err!("Error retrieving attachment : {}, url : {}", &err, &url))?;
        Ok(Some(Doc {
            url: url.to_owned(),
            content: DocContent::Other(buf),
        }))
    }
}

struct NewRepoWriter<'a> {
    update_repo: UpdateRepo,
    doc_repo: DocRepo,
    tag_repo: TagRepo,
    data: &'a RwLock<Data>,
    write_avoidance_buffer: RefCell<Vec<u8>>,
}
impl<'a> NewRepoWriter<'a> {
    fn new(new_repo: &Path, data: &'a RwLock<Data>) -> Result<Self> {
        let update_repo = UpdateRepo::new(new_repo.join("url"))?;
        let doc_repo = DocRepo::new(new_repo.join("url"))?;
        let tag_repo = TagRepo::new(new_repo.join("tag"))?;
        Ok(Self {
            update_repo,
            doc_repo,
            tag_repo,
            data,
            write_avoidance_buffer: RefCell::new(Vec::new()),
        })
    }

    fn write_update(&self, url: &Url, updated_at: &str, change: &str, category: Option<&str>) -> Result<()> {
        const DATE_FORMAT: &str = "%I:%M%p, %d %B %Y"; // 12:00pm, 27 March 2025
        if let Ok(ts) = chrono::NaiveDateTime::parse_from_str(updated_at, DATE_FORMAT)
            .map(|dt| {
                chrono_tz::Europe::London
                    .from_local_datetime(&dt)
                    .unwrap()
                    .fixed_offset()
            })
            .context("parsing timestamp")
        {
            let update_res = self.update_repo.create(url.clone().into(), ts, change).map(|update| {
                println!("Wrote update to update repo");
                if let Ok(mut data) = self.data.write() {
                    data.append_update(update.into_inner());
                }
            });

            if update_res.is_ok() || update_res.as_ref().unwrap_err().kind() == io::ErrorKind::AlreadyExists {
                self.tag_repo
                    .tag_update(
                        category.unwrap_or("unknown").to_owned(),
                        (url.to_owned().into(), ts).into(),
                    )
                    .map(|tag| {
                        for e in tag.into_events() {
                            self.handle_tag_event(e);
                        }
                    })?;
            }
            update_res?;
        }
        Ok(())
    }

    fn write_doc(
        &self,
        url: Url,
        ts: chrono::DateTime<chrono::FixedOffset>,
        content: impl AsRef<[u8]>,
    ) -> io::Result<()> {
        self.doc_repo
            .create(url.into(), ts, &mut self.write_avoidance_buffer.borrow_mut())
            .and_then(|mut doc| doc.write_all(content.as_ref()).and_then(|_| doc.done()))
            .map(|doc| {
                println!("Wrote doc to doc repo");
                for e in doc.into_events() {
                    self.handle_doc_event(e);
                }
            })
    }

    pub(crate) fn handle_tag_event(&self, e: TagEvent) {
        match e {
            TagEvent::UpdateTagged { tag, update_ref } => {
                if let Ok(mut data) = self.data.write() {
                    data.add_tag(update_ref, Arc::new(tag));
                }
            }
            TagEvent::TagCreated { tag: _ } => {}
        }
    }

    pub(crate) fn handle_doc_event(&self, e: DocEvent) {
        match e {
            DocEvent::Created { url: _ } => {}
            DocEvent::Updated { url: _, timestamp: _ } => {}
            DocEvent::Deleted { url: _, timestamp: _ } => {}
        }
    }
}

#[cfg(test)]
mod test {
    // use super::UpdateEmailProcessor;
    // use super::{email_update::GovUkChange, git::CommitBuilder};
    // use anyhow::Result;
    // use git2::{Repository, Signature};
    // use std::path::Path;
    // use update_repo::doc::DocRepo;
    // use update_repo::tag::TagRepo;
    // use update_repo::update::UpdateRepo;

    // #[test]
    // fn test_obtain_changes() -> Result<()> {
    //     const REPO_DIR: &str = "tests/tmp/test_obtain_changes";
    //     let _ = std::fs::remove_dir_all(REPO_DIR);
    //     let repo = Repository::init_bare(REPO_DIR)?;
    //     let test_sig = Signature::now("name", "email")?;
    //     CommitBuilder::new(&repo, None)?.commit(&test_sig, &test_sig, "initial commit")?;
    //     // let oid = repo.treebuilder(None)?.write()?;
    //     // let tree = repo.find_tree(oid)?;
    //     // repo.commit(Some(GIT_REF), &test_sig, &test_sig, "initial commit", &tree, &[])?;
    //     let update_email_processor = UpdateEmailProcessor {
    //         in_dir: "".as_ref(),
    //         out_dir: "".as_ref(),
    //         git_repo: repo,
    //         git_reference: "",
    //         update_repo: UpdateRepo::new("testtemprepo")?,
    //         doc_repo: DocRepo::new("testtemprepo")?,
    //         tag_repo: TagRepo::new("testtemprepo")?,
    //     };
    //     let commit = update_email_processor.handle_change(
    //         &GovUkChange {
    //             url: "https://www.gov.uk/government/consultations/bus-services-act-2017-bus-open-data".parse()?,
    //             change: "testing the stuff".to_owned(),
    //             updated_at: "some time".to_owned(),
    //             category: Some("Test Category".to_owned()),
    //         },
    //         None,
    //     )?;
    //     update_email_processor
    //         .git_repo
    //         .reference("refs/heads/main", commit.id(), false, "log_message")?;

    //     assert_eq!(commit.message(), Some("some time: testing the stuff [Test Category]"));
    //     assert_eq!(
    //         commit
    //             .tree()?
    //             .get_path(Path::new(
    //                 "government/consultations/bus-services-act-2017-bus-open-data.html"
    //             ))?
    //             .to_object(&update_email_processor.git_repo)?
    //             .as_blob()
    //             .unwrap()
    //             .size(),
    //         20929
    //     );
    //     Ok(())
    // }
}
