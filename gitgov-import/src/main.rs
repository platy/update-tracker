use std::{
    fs::remove_dir_all,
    io::{self, Read, Write},
    iter::successors,
    ops::AddAssign,
    str::from_utf8,
};

use anyhow::{ensure, format_err, Context, Result};
use extractor::Extractor;
use git2::Repository;
use update_repo::{
    doc::{DocEvent, DocRepo},
    tag::TagRepo,
    update::UpdateRepo,
    Url,
};

mod extractor;

fn main() -> Result<()> {
    let base_repo: &str = &dotenv::var("BASE_REPO")?;
    let tag_repo_base = &format!("{}/tag", base_repo);
    let url_repo_base: &str = &format!("{}/url", base_repo);
    let _ = remove_dir_all(tag_repo_base);
    let _ = remove_dir_all(url_repo_base);

    let repo = Repository::open(dotenv::var("GITGOV_REPO")?)?;
    let reference = repo.find_reference(&dotenv::var("GITGOV_REF")?)?;
    let last_commit = reference.peel_to_commit()?;

    let mut doc_repo = DocRepo::new(url_repo_base)?;
    let mut tag_repo = TagRepo::new(tag_repo_base)?;
    let mut update_repo = UpdateRepo::new(url_repo_base)?;

    let mut update_imports_skipped = 0;
    let mut updates_imported = 0;
    let mut doc_stats = DocImportStats::new();

    for commit in successors(Some(last_commit), |commit| commit.parents().next()) {
        if commit.author().email().unwrap() == "info@gov.uk" {
            let extractor = Extractor::new(&repo, &commit);
            doc_stats += import_docs_from_commit(&extractor, &mut doc_repo)
                .context(format!("Importing docs from {}", commit.id()))?;
            if let Err(e) = import_update_from_commit(&extractor, &mut tag_repo, &mut update_repo)
                .context(format!("Importing tag from {}", commit.id()))
            {
                println!("Error importing tag : {:? }\n", e);
                update_imports_skipped += 1;
            } else {
                updates_imported += 1;
            }
        } else {
            println!("Non-update commit : {}", commit.message().unwrap());
        }

        let commit_date = chrono::TimeZone::timestamp(
            &chrono::FixedOffset::east(60 * commit.time().offset_minutes()),
            commit.time().seconds(),
            0,
        )
        .date();

        print!(
            "{}: Imported: {} docs: {} new, {} updated, {} deleted, {} updates. {} skipped updates. {} deleted docs\r",
            commit_date,
            doc_stats.docs_imported,
            doc_stats.events_new,
            doc_stats.events_updated,
            doc_stats.events_deleted,
            updates_imported,
            update_imports_skipped,
            doc_stats.skip_deleted,
        );
        io::stdout().flush().unwrap();
    }
    println!("{} docs imported", doc_stats.docs_imported);
    println!("{} updates imported", updates_imported);
    println!("{} errors importing updates", update_imports_skipped);
    println!("{} deleted docs skipped", doc_stats.skip_deleted);

    Ok(())
}

/// Import a tag into the tag repo from the commit. If the commit only has one file it is easy, but if it has more, we need to find which of the files matches the update in the commit
fn import_update_from_commit(
    extractor: &Extractor,
    tag_repo: &mut TagRepo,
    update_repo: &mut UpdateRepo,
) -> Result<()> {
    use chrono::Timelike;

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

    let _tag = tag_repo
        .tag_update(tag.to_owned(), (url.clone(), ts2).into())
        .context("Tagging update in repo")?;
    let _update = update_repo
        .ensure(url, ts2, &change)
        .context("Creating update in repo")?;
    Ok(())
}

fn import_docs_from_commit(extractor: &Extractor, doc_repo: &mut DocRepo) -> Result<DocImportStats> {
    let mut docs_imported = 0;
    let mut events_new = 0;
    let mut events_updated = 0;
    let mut events_deleted = 0;
    let ts = extractor.retrieved_at();
    let (doc_versions, skip_deleted) = extractor.doc_versions().context("loading doc versions")?;
    for (url, content) in doc_versions {
        let url: Url = url.into();
        match doc_repo.create(url.clone(), ts) {
            Ok(mut writer) => {
                writer.write_all(content.as_bytes())?;
                let update = writer.done()?;
                for event in update.into_events() {
                    match event {
                        DocEvent::Created { .. } => events_new += 1,
                        DocEvent::Updated { .. } => events_updated += 1,
                        DocEvent::Deleted { .. } => events_deleted += 1,
                    }
                }
                docs_imported += 1;
                Ok(())
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                let existing = doc_repo.ensure_version(url.clone(), ts)?;
                let mut existing_data: Vec<u8> = vec![];
                doc_repo.open(&existing)?.read_to_end(&mut existing_data)?;
                if existing_data == content.as_bytes() {
                    println!("Doc version already exists {}", &existing);
                    Ok(())
                } else {
                    let diff = prettydiff::diff_lines(from_utf8(&existing_data)?, content.as_str());
                    Err(format_err!(
                        "Doc version exists for {}/{} with different content : {}",
                        &url.as_str(),
                        &ts,
                        diff,
                    ))
                }
            }
            Err(err) => Err(err).context("error writing doc version"),
        }?;
    }
    Ok(DocImportStats {
        docs_imported,
        skip_deleted,
        events_new,
        events_updated,
        events_deleted,
    })
}

struct DocImportStats {
    docs_imported: u16,
    skip_deleted: u16,
    events_new: u16,
    events_updated: u16,
    events_deleted: u16,
}

impl DocImportStats {
    fn new() -> Self {
        Self {
            docs_imported: 0,
            skip_deleted: 0,
            events_new: 0,
            events_updated: 0,
            events_deleted: 0,
        }
    }
}

impl AddAssign for DocImportStats {
    fn add_assign(
        &mut self,
        Self {
            docs_imported,
            skip_deleted,
            events_new,
            events_updated,
            events_deleted,
        }: Self,
    ) {
        self.docs_imported += docs_imported;
        self.skip_deleted += skip_deleted;
        self.events_new += events_new;
        self.events_updated += events_updated;
        self.events_deleted += events_deleted;
    }
}
