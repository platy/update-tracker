//! Test that the document fetch and strip process is consistent
//!
//! The document format is changed on the backend for many reasons, and we need to ensure that the document fetch and strip process is consistent.
//! The test can be synced with the backend by fetching new version of the docs, eg.
//!
//! ```sh
//! cargo run --bin fetch https://www.gov.uk/change-name-deed-poll/make-an-adult-deed-poll govdiff_server/tests/govuk
//! ```
//!

use govdiff_server::ingress::retrieve_doc;
use pretty_assertions::assert_eq;
use update_repo::doc::content::*;

macro_rules! assert_doc {
    ($doc:expr, $url:expr, $body:expr) => {
        let doc = $doc;
        let url = $url;
        assert_eq!(doc.url.as_str(), url);
        if let DocContent::DiffableHtml(content, _, _) = &doc.content {
            let diff = html_diff::get_differences($body, &content);
            assert!(
                diff.is_empty(),
                "Found differences in file at url {} : {}",
                url,
                diff.iter().map(|diff| diff.to_string()).collect::<String>(),
            );
        } else {
            panic!("Fail")
        }
    };
}

#[test]
fn fetch_and_strip_doc() {
    let doc = retrieve_doc(
        &"https://www.gov.uk/change-name-deed-poll/make-an-adult-deed-poll"
            .parse()
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_doc!(
        &doc,
        "https://www.gov.uk/change-name-deed-poll/make-an-adult-deed-poll",
        include_str!("govuk/change-name-deed-poll/make-an-adult-deed-poll.html")
    );
    assert_eq!(
        doc.content.history().unwrap(),
        vec![], // no history due to the type of doc
    );
}

#[test]
fn fetch_and_strip_doc_with_attachments_and_history() {
    let doc = retrieve_doc(
        &"https://www.gov.uk/government/consultations/bus-services-act-2017-bus-open-data"
            .parse()
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_doc!(
        &doc,
        "https://www.gov.uk/government/consultations/bus-services-act-2017-bus-open-data",
        include_str!("govuk/government/consultations/bus-services-act-2017-bus-open-data.html")
    );
    assert_eq!(doc.content.attachments().unwrap(),
        vec![
            "https://assets.publishing.service.gov.uk/media/5ca4b38d40f0b625eb19dba5/bus-open-data-consultation-response.pdf".parse().unwrap(),
            "https://www.gov.uk/government/consultations/bus-services-act-2017-bus-open-data/bus-services-act-2017-bus-open-data-html".parse().unwrap(),
            "https://assets.publishing.service.gov.uk/media/5b3de141ed915d39d5dab509/bus-services-act-2017-open-data-consultation.pdf".parse().unwrap(),
            "https://assets.publishing.service.gov.uk/media/5b3de1eded915d39e7ed0e7e/bus-open-data-case-for-change.pdf".parse().unwrap(),
        ]);
    assert_eq!(
        doc.content.history().unwrap(),
        vec![
            DocUpdate::new(
                "2019-03-27T15:21:23.000+00:00".parse().unwrap(),
                "Document revised for missing data in table 3."
            ),
            DocUpdate::new(
                "2019-03-26T00:15:02.000+00:00".parse().unwrap(),
                "Consultation response released."
            ),
            DocUpdate::new("2018-07-05T00:15:03.000+01:00".parse().unwrap(), "First published."),
        ]
    );
}

#[test]
fn fetch_file() {
    let doc = retrieve_doc(
        &"https://assets.publishing.service.gov.uk/government/uploads/system/uploads/attachment_data/file/722576/bus-open-data-case-for-change.pdf".parse().unwrap(),
    )
    .unwrap().unwrap();
    assert_file(
        &doc,
        "https://assets.publishing.service.gov.uk/government/uploads/system/uploads/attachment_data/file/722576/bus-open-data-case-for-change.pdf",
        include_bytes!(
            "govuk/government/uploads/system/uploads/attachment_data/file/722576/bus-open-data-case-for-change.pdf"
        ),
    );
    assert!(doc.content.attachments().is_none());
}

fn assert_file(doc: &Doc, url: &str, body: &[u8]) {
    assert_eq!(doc.url.as_str(), url,);
    if let DocContent::Other(content) = &doc.content {
        assert_eq!(content.as_slice(), body);
    } else {
        panic!("Fail")
    }
}
