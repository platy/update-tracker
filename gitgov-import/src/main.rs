use std::{
    fs::remove_dir_all,
    io::{self, Read, Write},
    str::from_utf8,
};

use anyhow::{ensure, format_err, Context, Result};
use chrono::Timelike;
use extractor::Extractor;
use git2::Repository;
use update_tracker::{doc::DocRepo, tag::TagRepo, update::UpdateRepo, Url};

mod extractor;

fn main() -> Result<()> {
    let base_repo: &str = &dotenv::var("BASE_REPO")?;
    let tag_repo_base = &format!("{}/tag", base_repo);
    let doc_repo_base: &str = &format!("{}/doc", base_repo);
    let update_repo_base: &str = &format!("{}/update", base_repo);
    let _ = remove_dir_all(tag_repo_base);
    let _ = remove_dir_all(doc_repo_base);
    let _ = remove_dir_all(update_repo_base);

    let repo = Repository::open(dotenv::var("GITGOV_REPO")?)?;
    let reference = repo.find_reference(&dotenv::var("GITGOV_REF")?)?;
    let mut commit = reference.peel_to_commit()?;

    let mut doc_repo = DocRepo::new(doc_repo_base)?;
    let mut tag_repo = TagRepo::new(tag_repo_base)?;
    let mut update_repo = UpdateRepo::new(update_repo_base)?;

    let mut update_imports_skipped = 0;
    let mut deleted_docs_skipped = 0;
    let mut docs_imported = 0;
    let mut updates_imported = 0;

    loop {
        if commit.author().email().unwrap() == "info@gov.uk" {
            let extractor = Extractor::new(&repo, &commit);
            let (doc_count, skipped_deleted) = import_docs_from_commit(&extractor, &mut doc_repo)
                .context(format!("Importing docs from {}", commit.id()))?;
            docs_imported += doc_count;
            deleted_docs_skipped += skipped_deleted;
            if let Err(e) = import_update_from_commit(&extractor, &mut tag_repo, &mut update_repo)
                .context(format!("Importing tag from {}", commit.id()))
            {
                println!("Error importing tag : {:? }", e);
                update_imports_skipped += 1;
            } else {
                updates_imported += 1;
            }
        } else {
            println!("Non-update commit : {}", commit.message().unwrap());
        }

        print!(
            "Imported: {} docs, {} updates. {} skipped updates. {} deleted docs\r",
            docs_imported, updates_imported, update_imports_skipped, deleted_docs_skipped
        );
        io::stdout().flush().unwrap();

        if let Some(parent) = commit.parents().next() {
            commit = parent;
        } else {
            break;
        }
    }
    println!("{} docs imported", docs_imported);
    println!("{} updates imported", updates_imported);
    println!("{} errors importing updates", update_imports_skipped);
    println!("{} deleted docs skipped", deleted_docs_skipped);

    Ok(())
}

/// Import a tag into the tag repo from the commit. If the commit only has one file it is easy, but if it has more, we need to find which of the files matches the update in the commit
fn import_update_from_commit(
    extractor: &Extractor,
    tag_repo: &mut TagRepo,
    update_repo: &mut UpdateRepo,
) -> Result<()> {
    let ts1 = extractor.updated_at()?;
    let change = extractor.message()?;
    let tag = extractor.tag().unwrap_or("Unknown");

    let (url, ts2) = extractor
        .main_doc_version()
        .context("Finding the main doc version in the update")?;
    let url: Url = url.into();

    ensure!(
        ts1 == ts2.with_second(0).unwrap(),
        "expected {} == {}",
        ts1,
        ts2.with_second(0).unwrap()
    );

    let (_tag, _events) = tag_repo
        .tag_update(tag.to_owned(), (url.clone(), ts2).into())
        .context("Tagging update in repo")?;
    let (_update, _events) = update_repo
        .ensure(url, ts2, &change)
        .context("Creating update in repo")?;
    Ok(())
}

fn import_docs_from_commit(extractor: &Extractor, doc_repo: &mut DocRepo) -> Result<(u32, u32)> {
    let mut count = 0;
    let ts = extractor.retrieved_at();
    let (doc_versions, skip_deleted) = extractor.doc_versions().context("loading doc versions")?;
    for (url, content) in doc_versions {
        let url: Url = url.into();
        match doc_repo.create(url.clone(), ts) {
            Ok(mut writer) => {
                writer.write_all(content.as_bytes())?;
                let (_update, _events) = writer.done()?;
                count += 1;
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
    Ok((count, skip_deleted))
}
