use std::env;

use chrono::Utc;
use update_repo::doc::{content::sanitise_doc, DocRepo};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args();
    let _ = args.next().unwrap();
    let source_path = args.next().expect("no source path");
    let dest_path = args.next().expect("no dest path");

    let source_doc_repo = DocRepo::new(source_path)?;
    let dest_doc_repo = DocRepo::new(dest_path)?;

    let mut write_avoidance_buffer = Vec::new();
    let mut buf = Vec::new();
    let mut last_wrote = 0;
    let mut last_nl = 0;
    let mut count = 0;
    for res in source_doc_repo
        .list_all(&"https://www.gov.uk/".parse().unwrap())
        .unwrap()
    {
        count += 1;
        let doc_ver = res?;
        let ts = Utc::now().timestamp_millis();
        if ts * 60 / 1000 > last_wrote {
            print!("\rCopying #{} {}", count, doc_ver);
            last_wrote = ts * 60 / 1000;
        }
        if ts / 10_000 > last_nl {
            println!();
            last_nl = ts / 10_000;
        }

        let mut read = source_doc_repo.open(&doc_ver)?;
        let mut write =
            dest_doc_repo.create(doc_ver.url().clone(), *doc_ver.timestamp(), &mut write_avoidance_buffer)?;
        sanitise_doc(&mut read, &mut write, &mut buf)?;
        let _ = write.done()?;
    }
    Ok(())
}
