use anyhow::{bail, format_err, Context, Result};
use std::io::copy;
use ureq::get;
use url::Url;

pub mod doc;
pub mod email_update;
pub use doc::{Doc, DocContent};
pub mod git;

use dotenv::dotenv;
use file_lock::FileLock;
use git2::{Commit, Repository, Signature};
use self::{email_update::GovUkChange, git::CommitBuilder};
use std::{
    collections::VecDeque,
    fs,
    io::Read,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

pub fn run() -> Result<()> {
    dotenv()?;
    let govuk_emails_inbox = dotenv::var("INBOX")?;
    const ARCHIVE_DIR: &str = "outbox";
    let repo_path = dotenv::var("REPO")?;
    let reference = dotenv::var("REF")?;
    fs::create_dir_all(&govuk_emails_inbox)
        .context(format!("Error trying to create dir {}", &govuk_emails_inbox))?;
    fs::create_dir_all(ARCHIVE_DIR).context(format!("Error trying to create dir {}", ARCHIVE_DIR))?;

    push(&repo_path)?;

    loop {
        let count = process_updates_in_dir(&govuk_emails_inbox, ARCHIVE_DIR, &repo_path, &reference)
            .expect("the processing fails, the repo may be unclean");
        if count > 0 {
            println!("Processed {} update emails, pushing", count);
            push(&repo_path).unwrap_or_else(|err| println!("Push failed : {}", err));
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn push(repo_base: impl AsRef<Path>) -> Result<()> {
    println!("Pushing to github");
    let mut remote_callbacks = git2::RemoteCallbacks::new();
    remote_callbacks.credentials(|_url, username_from_url, _allowed_types| {
        git2::Cred::ssh_key(
            username_from_url.unwrap(),
            None,
            std::path::Path::new(&format!("{}/.ssh/id_rsa", std::env::var("HOME").unwrap())),
            None,
        )
    });
    let repo = Repository::open(repo_base).context("Opening repo")?;
    let mut remote = repo.find_remote("origin")?;
    remote.push(
        &["refs/heads/main"],
        Some(git2::PushOptions::new().remote_callbacks(remote_callbacks)),
    )?;
    Ok(())
}

fn process_updates_in_dir(
    in_dir: impl AsRef<Path>,
    out_dir: impl AsRef<Path>,
    repo: impl AsRef<Path>,
    reference: &str,
) -> Result<u32> {
    let mut count = 0;
    for to_inbox in fs::read_dir(in_dir)? {
        let to_inbox = to_inbox?;
        if to_inbox.metadata()?.is_dir() {
            for email in fs::read_dir(to_inbox.path())? {
                let email = email?;
                println!("Processing {:?}", email);
                if !(process_email_update_file(to_inbox.file_name(), &email, &out_dir, &repo, reference).context(
                    format!("Failed processing {}", email.path().to_str().unwrap_or_default()),
                )?) {
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

fn process_email_update_file(
    to_dir_name: impl AsRef<Path>,
    dir_entry: &fs::DirEntry,
    out_dir: impl AsRef<Path>,
    repo_base: impl AsRef<Path>,
    reference: &str,
) -> Result<bool> {
    let data = {
        let mut lock = FileLock::lock(dir_entry.path().to_str().context("error")?, true, false)
            .context("Locking file email file")?;
        let mut bytes = Vec::with_capacity(lock.file.metadata().map(|m| m.len() as usize + 1).unwrap_or(0));
        lock.file.read_to_end(&mut bytes).context("Reading email file")?;
        bytes
    };
    let updates = GovUkChange::from_eml(&String::from_utf8(data)?).context("Parsing email")?;
    let repo = Repository::open(repo_base).context("Opening repo")?;
    let mut parent = Some(repo.find_reference(reference)?.peel_to_commit()?);
    for change in &updates {
        match handle_change(change, &repo, parent) {
            Ok(p) => parent = Some(p),
            Err(err) => {
                eprintln!("Error processing change: {:?}: {}", change, &err);
                return Ok(false);
            }
        }
    }
    // successfully handled, 'commit' the new commits by updating the reference and then move email to outbox
    if let Some(commit) = parent {
        let _ref = repo.reference(
            reference,
            commit.id(),
            true,
            &format!("Added updates from {:?}", dir_entry.path()),
        )?;
    }
    let done_path = out_dir.as_ref().join(to_dir_name).join(dir_entry.file_name());
    fs::create_dir_all(done_path.parent().unwrap()).context("Creating outbox dir")?;
    fs::rename(dir_entry.path(), &done_path).context(format!(
        "Renaming file {} to {}",
        dir_entry.path().to_str().unwrap_or_default(),
        &done_path.to_str().unwrap_or_default()
    ))?;
    Ok(true)
}

fn handle_change<'repo>(
    GovUkChange {
        url,
        change,
        updated_at,
        category,
    }: &GovUkChange,
    repo: &'repo Repository,
    parent: Option<Commit<'repo>>,
) -> Result<Commit<'repo>> {
    let mut commit_builder = CommitBuilder::new(repo, parent)?;

    fetch_change(url, |path, bytes| {
        // write the blob
        let oid = repo.blob(bytes)?;
        commit_builder.add_to_tree(path.to_str().unwrap(), oid, 0o100644)
    })?;

    let message = format!(
        "{}: {}{}",
        updated_at,
        change,
        category.as_ref().map(|c| format!(" [{}]", c)).unwrap_or_default()
    );
    let govuk_sig = Signature::now("Gov.uk", "info@gov.uk")?;
    let gitgov_sig = Signature::now("Gitgov", "gitgov@njk.onl")?;
    Ok(commit_builder.commit(&govuk_sig, &gitgov_sig, &message)?)
}

fn fetch_change(url: &Url, mut write_out: impl FnMut(PathBuf, &[u8]) -> Result<()>) -> Result<()> {
    let mut urls = VecDeque::new();
    urls.push_back(url.to_owned());

    while let Some(url) = urls.pop_front() {
        if url.host_str() != Some("www.gov.uk") {
            println!("Ignoring link to offsite document : {}", &url);
            continue;
        }
        let doc = retrieve_doc(&url)?;
        urls.extend(doc.content.attachments().unwrap_or_default().iter().cloned());

        let mut path = PathBuf::from(doc.url.path());
        if doc.content.is_html() {
            assert!(path.set_extension("html"));
        }
        println!("Writing doc to : {}", path.to_str().unwrap());
        write_out(path, doc.content.as_bytes())?
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use super::handle_change;
    use anyhow::Result;
    use git2::{Repository, Signature};
    use super::{email_update::GovUkChange, git::CommitBuilder};
    use std::path::Path;

    #[test]
    fn test_obtain_changes() -> Result<()> {
        const REPO_DIR: &str = "tests/tmp/test_obtain_changes";
        let _ = std::fs::remove_dir_all(REPO_DIR);
        let repo = Repository::init_bare(REPO_DIR)?;
        let test_sig = Signature::now("name", "email")?;
        CommitBuilder::new(&repo, None)?.commit(&test_sig, &test_sig, "initial commit")?;
        // let oid = repo.treebuilder(None)?.write()?;
        // let tree = repo.find_tree(oid)?;
        // repo.commit(Some(GIT_REF), &test_sig, &test_sig, "initial commit", &tree, &[])?;
        let commit = handle_change(
            &GovUkChange {
                url: "https://www.gov.uk/government/consultations/bus-services-act-2017-bus-open-data".parse()?,
                change: "testing the stuff".to_owned(),
                updated_at: "some time".to_owned(),
                category: Some("Test Category".to_owned()),
            },
            &repo,
            None,
        )?;
        repo.reference("refs/heads/main", commit.id(), false, "log_message")?;

        assert_eq!(commit.message(), Some("some time: testing the stuff [Test Category]"));
        assert_eq!(
            commit
                .tree()?
                .get_path(Path::new(
                    "government/consultations/bus-services-act-2017-bus-open-data.html"
                ))?
                .to_object(&repo)?
                .as_blob()
                .unwrap()
                .size(),
            20929
        );
        Ok(())
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
