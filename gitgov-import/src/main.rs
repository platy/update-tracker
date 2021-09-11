use std::{fs::remove_dir_all, io, str::from_utf8};

use anyhow::{ensure, format_err, Context, Result};
use extractor::Extractor;
use git2::Repository;
use io::{Read, Write};
use update_tracker::{doc::DocRepo, tag::TagRepo, update::UpdateRepo};

mod extractor;

fn main() -> Result<()> {
    const TAG_REPO_BASE: &str = "./out/tag";
    const DOC_REPO_BASE: &str = "./out/doc";
    const UPDATE_REPO_BASE: &str = "./out/update";
    let _ = remove_dir_all(TAG_REPO_BASE);
    let _ = remove_dir_all(DOC_REPO_BASE);
    let _ = remove_dir_all(UPDATE_REPO_BASE);

    let repo = Repository::open(dotenv::var("GITGOV_REPO")?)?;
    let reference = repo.find_reference(&dotenv::var("GITGOV_REF")?)?;
    let mut commit = reference.peel_to_commit()?;

    let mut doc_repo = DocRepo::new(DOC_REPO_BASE)?;
    let mut tag_repo = TagRepo::new(TAG_REPO_BASE)?;
    let mut update_repo = UpdateRepo::new(UPDATE_REPO_BASE)?;

    let mut tag_imports_skipped = 0;

    loop {
        if commit.author().email().unwrap() == "info@gov.uk" {
            let extractor = Extractor::new(&repo, &commit);
            import_docs_from_commit(&extractor, &mut doc_repo)
                .context(format!("Importing docs from {}", commit.id()))?;
            if let Err(e) = import_tag_from_commit(&extractor, &mut tag_repo, &mut update_repo).context(format!("Importing tag from {}", commit.id())) {
                println!("Error importing tag : {:? }", e);
                tag_imports_skipped += 1;
            }
        } else {
            println!("Non-update commit : {}", commit.message().unwrap());
        }

        if let Some(parent) = commit.parents().next() {
            commit = parent;
        } else {
            break;
        }
    }
    println!("{} errors importing tags", tag_imports_skipped);

    Ok(())
}

/// INPRROGESS Import a tag into the tag repo from the commit. If the commit only has one file it is easy, but if it has more, we need to find which of the files matches the update in the commit
fn import_tag_from_commit(extractor: &Extractor, tag_repo: &mut TagRepo, update_repo: &mut UpdateRepo) -> Result<()> {
    let ts1 = extractor.updated_at()?;
    let change = extractor.message()?;
    let tag = extractor.tag().unwrap_or("Unknown");

    let (url, ts2) = extractor.main_doc_version()?;

    ensure!(ts1 == ts2);

    let (_tag, _events) = tag_repo.tag_update(tag.to_owned(), (url.clone(), ts2).into())?;
    let (_update, _events) = update_repo.create(url, ts2, &change)?;
    Ok(())
}

fn import_docs_from_commit(extractor: &Extractor, doc_repo: &mut DocRepo) -> Result<()> {
    let ts = extractor.retrieved_at();
    for (url, content) in extractor.doc_versions().context("loading doc versions")? {
        match doc_repo.create(url.clone(), ts) {
            Ok(mut writer) => {
                writer.write_all(content.as_bytes())?;
                let (update, _events) = writer.done()?;
                println!("create {}", &update);
                Ok(())
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                let existing = doc_repo.ensure_version(url.clone(), ts)?;
                let mut existing_data: Vec<u8> = vec![];
                doc_repo.open(&existing)?.read_to_end(&mut existing_data)?;
                if existing_data == content.as_bytes() {
                    println!("exists {}", &existing);
                    Ok(())
                } else {
                    let diff = prettydiff::diff_lines(from_utf8(&existing_data)?, content.as_str());
                    Err(format_err!(
                        "Update exists for {}/{} with different content : {}",
                        &url.as_str(),
                        &ts,
                        diff,
                    ))
                }
            }
            Err(err) => Err(err).context("error writing update"),
        }?;
    }
    Ok(())
}
