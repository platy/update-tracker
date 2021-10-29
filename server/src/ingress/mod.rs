use anyhow::{bail, format_err, Context, Result};
use chrono::{Offset, TimeZone, Utc};
use std::{
    io::{self, copy, Write},
    sync::{Arc, RwLock},
};
use update_repo::{
    doc::{DocEvent, DocRepo},
    tag::{TagEvent, TagRepo},
    update::UpdateRepo,
};
use ureq::get;
use url::Url;

pub mod doc;
pub mod email_update;
pub use doc::{Doc, DocContent};
pub mod git;

use self::{
    email_update::GovUkChange,
    git::{GitRepoTransaction, GitRepoWriter},
};
use crate::data::Data;
use dotenv::dotenv;
use file_lock::FileLock;

use std::{
    collections::VecDeque,
    fs,
    io::Read,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

pub fn run(data: Arc<RwLock<Data>>) -> Result<()> {
    dotenv()?;
    let govuk_emails_inbox = dotenv::var("INBOX")?;
    const ARCHIVE_DIR: &str = "outbox";
    let git_repo_path = dotenv::var("GIT_REPO")?;
    let git_reference = dotenv::var("GIT_REF")?;
    let new_repo_path = dotenv::var("NEW_REPO")?;
    fs::create_dir_all(&govuk_emails_inbox).context(format!("Error trying to create dir {}", &govuk_emails_inbox))?;
    fs::create_dir_all(ARCHIVE_DIR).context(format!("Error trying to create dir {}", ARCHIVE_DIR))?;

    git::push(&git_repo_path)?;

    loop {
        let mut update_email_processor = UpdateEmailProcessor::new(
            govuk_emails_inbox.as_ref(),
            ARCHIVE_DIR.as_ref(),
            git_repo_path.as_ref(),
            &git_reference,
            new_repo_path.as_ref(),
            &data,
        )?;
        let count = update_email_processor
            .process_updates()
            .expect("the processing fails, the repo may be unclean");
        if count > 0 {
            println!("Processed {} update emails, pushing", count);
            git::push(&git_repo_path).unwrap_or_else(|err| println!("Push failed : {}", err));
        }
        thread::sleep(Duration::from_secs(1));
    }
}

struct UpdateEmailProcessor<'a> {
    in_dir: &'a Path,
    out_dir: &'a Path,
    git: GitRepoWriter<'a>,
    new: NewRepoWriter<'a>,
}

impl<'a> UpdateEmailProcessor<'a> {
    fn new(
        in_dir: &'a Path,
        out_dir: &'a Path,
        git_repo: &'a Path,
        git_reference: &'a str,
        new_repo: &Path,
        data: &'a RwLock<Data>,
    ) -> Result<Self> {
        Ok(Self {
            in_dir,
            out_dir,
            git: GitRepoWriter::new(git_repo, git_reference)?,
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
        let data = {
            let mut lock = FileLock::lock(dir_entry.path().to_str().context("error")?, true, false)
                .context("Locking file email file")?;
            let mut bytes = Vec::with_capacity(lock.file.metadata().map(|m| m.len() as usize + 1).unwrap_or(0));
            lock.file.read_to_end(&mut bytes).context("Reading email file")?;
            bytes
        };
        let updates = GovUkChange::from_eml(&String::from_utf8(data)?).context("Parsing email")?;
        let mut git_transaction = self.git.start_transaction()?;
        for change in &updates {
            if let Err(err) = self.handle_change(change, &mut git_transaction) {
                eprintln!("Error processing change: {:?}: {}", change, &err);
                return Ok(false);
            }
        }
        // successfully handled, 'commit' the new commits by updating the reference and then move email to outbox
        git_transaction.commit(&format!("Added updates from {:?}", dir_entry.path()))?;
        let done_path = self.out_dir.join(to_dir_name).join(dir_entry.file_name());
        fs::create_dir_all(done_path.parent().unwrap()).context("Creating outbox dir")?;
        fs::rename(dir_entry.path(), &done_path).context(format!(
            "Renaming file {} to {}",
            dir_entry.path().to_str().unwrap_or_default(),
            &done_path.to_str().unwrap_or_default()
        ))?;
        Ok(true)
    }

    fn handle_change<'repo>(
        &'repo self,
        GovUkChange {
            url,
            change,
            updated_at,
            category,
        }: &GovUkChange,
        git_transaction: &mut GitRepoTransaction,
    ) -> Result<()> {
        if let Err(err) = self.new.write_update(url, updated_at, change, category.as_deref()) {
            println!("Error writign to update repo {}", err);
        }

        let mut commit_builder = git_transaction.start_change()?;

        for res in FetchDocs::fetch(url.clone()) {
            let (path, content) = res?;
            commit_builder.add_doc(&path, &content)?;

            let mut url = url.clone();
            url.set_path(path.to_str().unwrap());
            let ts = Utc::now();
            let ts = ts.with_timezone(&ts.offset().fix());
            if let Err(err) = self.new.write_doc(url, ts, content) {
                println!("Error writign to doc repo {}", err)
            }
        }

        commit_builder.commit_update(updated_at, change, category.as_deref())?;
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

    fn fetch_doc(&mut self, url: Url) -> Result<(PathBuf, DocContent)> {
        let doc = retrieve_doc(&url)?;
        self.urls
            .extend(doc.content.attachments().unwrap_or_default().iter().cloned());
        let mut path = PathBuf::from(doc.url.path());
        if doc.content.is_html() {
            assert!(path.set_extension("html"));
        }
        println!("Writing doc to : {}", path.to_str().unwrap());
        Ok((path, doc.content))
    }
}

impl Iterator for FetchDocs {
    type Item = Result<(PathBuf, DocContent)>;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(url) = self.urls.pop_front() {
            if url.host_str() != Some("www.gov.uk") {
                println!("Ignoring link to offsite document : {}", &url);
                continue;
            }
            return Some(self.fetch_doc(url));
        }
        None
    }
}

pub fn retrieve_doc(url: &Url) -> Result<Doc> {
    // TODO return the doc and the urls of attachments, probably remove async, I can just use a thread pool and worker queue
    println!("retrieving url : {}", url);
    let response = get(url.as_str()).call();
    if let Some(err) = response.synthetic_error() {
        bail!("Error retrieving : {}", err);
    }

    if response.content_type() == "text/html" {
        let content = response.into_string().with_context(|| url.clone())?;
        let doc = Doc {
            content: DocContent::html(&content, Some(url))?,
            url: url.to_owned(),
        };

        Ok(doc)
    } else {
        let mut reader = response.into_reader();
        let mut buf = vec![];
        copy(&mut reader, &mut buf)
            .map_err(|err| format_err!("Error retrieving attachment : {}, url : {}", &err, &url))?;
        Ok(Doc {
            url: url.to_owned(),
            content: DocContent::Other(buf),
        })
    }
}

struct NewRepoWriter<'a> {
    update_repo: UpdateRepo,
    doc_repo: DocRepo,
    tag_repo: TagRepo,
    data: &'a RwLock<Data>,
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
        })
    }

    fn write_update(&self, url: &Url, updated_at: &str, change: &str, category: Option<&str>) -> Result<()> {
        const DATE_FORMAT: &str = "%I:%M%p, %d %B %Y";
        if let Ok(ts) = chrono_tz::Europe::London
            .datetime_from_str(updated_at, DATE_FORMAT)
            .context("parsing timestamp")
        {
            let ts = ts.with_timezone(&ts.offset().fix());

            self.update_repo
                .create(url.clone().into(), ts, change)
                .and_then(|update| {
                    println!("Wrote update to update repo");
                    let update_ref = update.update_ref().clone();
                    if let Ok(mut data) = self.data.write() {
                        data.append_update(update.into_inner());
                    }
                    self.tag_repo
                        .tag_update(category.unwrap_or("unknown").to_owned(), update_ref)
                        .map(|tag| {
                            for e in tag.into_events() {
                                self.handle_tag_event(e);
                            }
                        })?;
                    Ok(())
                })?;
        }
        Ok(())
    }

    fn write_doc(&self, url: Url, ts: chrono::DateTime<chrono::FixedOffset>, content: DocContent) -> io::Result<()> {
        self.doc_repo
            .create(url.into(), ts)
            .and_then(|mut doc| doc.write_all(content.as_bytes()).and_then(|_| doc.done()))
            .map(|doc| {
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
